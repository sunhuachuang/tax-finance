//! Tool definitions and dispatch. Reads are open; writes create pending
//! records only. There is deliberately no tool that confirms anything.

use std::path::PathBuf;

use chrono::{NaiveDate, Utc};
use serde_json::{Value, json};
use taxcore::{
    AccountCode, BankTransaction, DocumentId, DocumentSource, DocumentStatus, EntrySource,
    EntryStatus, ExtractedInvoice, Money, TaxYear,
};
use taxingest::{
    DEFAULT_CONFIDENCE_FLOOR, DraftProposal, IncomingFile, Reading, ingest, propose_draft,
    propose_matches, record_reading, review_queue,
};
use taxrules::RuleSet;
use taxstore::Store;

pub struct Context {
    pub store: Store,
    pub data_dir: PathBuf,
    pub rules_dir: PathBuf,
}

impl Context {
    fn rules_for(&self, year: TaxYear) -> Result<RuleSet, String> {
        RuleSet::for_year(&self.rules_dir, "NZ", year).map_err(|e| e.to_string())
    }
}

pub fn tool_definitions() -> Vec<Value> {
    let invoice_shape = "Invoice fields as read from the document. Money is always \
        {\"cents\": <integer minor units>, \"currency\": \"NZD\"} — never floats. Shape: \
        {supplier_name, supplier_gst_number, invoice_number, invoice_date: \"YYYY-MM-DD\", \
        currency: \"NZD\", subtotal: Money|null, gst: Money|null, total: Money, \
        lines: [{description, quantity, amount: Money}]}. Report what the document says; \
        do not compute or correct any figure.";

    vec![
        json!({
            "name": "list_documents",
            "description": "List stored source documents, optionally by status.",
            "inputSchema": {"type": "object", "properties": {
                "status": {"type": "string", "enum": ["pending_extraction", "extracted", "needs_review", "ignored"],
                            "description": "Omit to list every document."}}},
        }),
        json!({
            "name": "get_document",
            "description": "One document plus its latest extraction, if any.",
            "inputSchema": {"type": "object", "required": ["document_id"], "properties": {
                "document_id": {"type": "string"}}},
        }),
        json!({
            "name": "review_queue",
            "description": "Everything waiting on the human: documents needing review and unconfirmed draft entries. Read-only; approval happens outside MCP.",
            "inputSchema": {"type": "object", "properties": {}},
        }),
        json!({
            "name": "list_entries",
            "description": "Ledger entries by status (default posted).",
            "inputSchema": {"type": "object", "properties": {
                "status": {"type": "string", "enum": ["draft", "posted", "voided", "reversed"]}}},
        }),
        json!({
            "name": "gst_return",
            "description": "Compute the GST101 figures for the filing period containing a date. Every box carries the entries and documents behind it.",
            "inputSchema": {"type": "object", "properties": {
                "date": {"type": "string", "description": "YYYY-MM-DD, any day inside the period. Defaults to today."},
                "frequency": {"type": "string", "enum": ["monthly", "two_monthly", "six_monthly"],
                               "description": "Defaults to the rule file's default (two-monthly)."}}},
        }),
        json!({
            "name": "ir3_summary",
            "description": "Business income, deductible expenses, net profit and the tax on it for a tax year, from the posted ledger.",
            "inputSchema": {"type": "object", "required": ["tax_year"], "properties": {
                "tax_year": {"type": "string", "description": "e.g. \"2025-26\" or \"2026\""}}},
        }),
        json!({
            "name": "unreconciled_bank",
            "description": "Bank statement rows no ledger entry accounts for yet.",
            "inputSchema": {"type": "object", "properties": {}},
        }),
        json!({
            "name": "propose_matches",
            "description": "Candidate pairings between unreconciled bank rows and extracted documents, strongest first. Proposals only — confirming a match stays with the human.",
            "inputSchema": {"type": "object", "properties": {
                "window_days": {"type": "integer", "description": "Date window, default 3."}}},
        }),
        json!({
            "name": "ingest_document",
            "description": "Store a document file (invoice, receipt, statement) into the pipeline. Content-addressed and deduplicated; the document enters as pending_extraction.",
            "inputSchema": {"type": "object", "required": ["path"], "properties": {
                "path": {"type": "string", "description": "Path to the file on disk."},
                "mime": {"type": "string", "description": "Defaults from the file extension."},
                "original_filename": {"type": "string"},
                "source": {"type": "string", "enum": ["upload", "photo"], "description": "Default upload."}}},
        }),
        json!({
            "name": "record_reading",
            "description": format!("Submit the structured fields you read off a document. The engine validates them arithmetically; errors or low confidence send the document to human review. {invoice_shape}"),
            "inputSchema": {"type": "object", "required": ["document_id", "extracted_by", "invoice"], "properties": {
                "document_id": {"type": "string"},
                "extracted_by": {"type": "string", "description": "The model doing the reading, e.g. \"claude-sonnet-5\"."},
                "confidence": {"type": "number", "description": "Your own 0-1 confidence. It can only demote the result, never promote it."},
                "invoice": {"type": "object", "description": "See tool description for the exact shape."}}},
        }),
        json!({
            "name": "propose_draft",
            "description": "Turn a cleanly-extracted purchase document into a DRAFT ledger entry (GST computed deterministically, provenance attached). The draft waits in the review queue; nothing you do here posts it.",
            "inputSchema": {"type": "object", "required": ["document_id", "expense_account", "funding_account", "model"], "properties": {
                "document_id": {"type": "string"},
                "expense_account": {"type": "string", "description": "Chart code, e.g. \"6100-office\"."},
                "funding_account": {"type": "string", "description": "Chart code, e.g. \"1010-bank\"."},
                "model": {"type": "string", "description": "Who is proposing the classification."}}},
        }),
        json!({
            "name": "import_bank_rows",
            "description": "Import bank statement rows (idempotent: re-importing overlapping ranges is safe). Rows arrive unreconciled; matching them to entries is proposed, then confirmed by the human.",
            "inputSchema": {"type": "object", "required": ["account", "batch", "rows"], "properties": {
                "account": {"type": "string"},
                "batch": {"type": "string", "description": "A label for this import, e.g. \"asb-2026-07.csv\"."},
                "rows": {"type": "array", "items": {"type": "object",
                    "required": ["date", "amount_cents", "description"],
                    "properties": {
                        "date": {"type": "string", "description": "YYYY-MM-DD"},
                        "amount_cents": {"type": "integer", "description": "Positive money in, negative money out."},
                        "description": {"type": "string"},
                        "reference": {"type": "string"}}}}}},
        }),
    ]
}

pub fn dispatch(ctx: &mut Context, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "list_documents" => list_documents(ctx, args),
        "get_document" => get_document(ctx, args),
        "review_queue" => {
            let queue = review_queue(&ctx.store).map_err(err)?;
            Ok(json!({
                "documents_needing_review": to_value(&queue.documents)?,
                "draft_entries": to_value(&queue.drafts)?,
            }))
        }
        "list_entries" => {
            let status = match args.get("status").and_then(Value::as_str) {
                None | Some("posted") => EntryStatus::Posted,
                Some("draft") => EntryStatus::Draft,
                Some("voided") => EntryStatus::Voided,
                Some("reversed") => EntryStatus::Reversed,
                Some(other) => return Err(format!("unknown entry status {other}")),
            };
            to_value(&ctx.store.entries_with_status(status).map_err(err)?)
        }
        "gst_return" => gst_return(ctx, args),
        "ir3_summary" => {
            let year: TaxYear = required_str(args, "tax_year")?.parse().map_err(err)?;
            let rules = ctx.rules_for(year)?;
            to_value(&taxreturn::ir3(&ctx.store, &rules, year).map_err(err)?)
        }
        "unreconciled_bank" => to_value(&ctx.store.unreconciled_bank_txns().map_err(err)?),
        "propose_matches" => {
            let window = args.get("window_days").and_then(Value::as_i64);
            to_value(&propose_matches(&ctx.store, window).map_err(err)?)
        }
        "ingest_document" => ingest_document(ctx, args),
        "record_reading" => record_reading_tool(ctx, args),
        "propose_draft" => propose_draft_tool(ctx, args),
        "import_bank_rows" => import_bank_rows(ctx, args),
        other => Err(format!(
            "unknown tool {other}. Note that approving, posting, reversing or voiding \
             entries is deliberately not available over MCP — ask the human."
        )),
    }
}

fn list_documents(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let statuses: &[DocumentStatus] = match args.get("status").and_then(Value::as_str) {
        Some(s) => &[parse_document_status(s)?],
        None => &[
            DocumentStatus::PendingExtraction,
            DocumentStatus::Extracted,
            DocumentStatus::NeedsReview,
            DocumentStatus::Ignored,
        ],
    };
    let mut documents = Vec::new();
    for status in statuses {
        documents.extend(ctx.store.documents_with_status(*status).map_err(err)?);
    }
    to_value(&documents)
}

fn get_document(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let id: DocumentId = required_str(args, "document_id")?.parse().map_err(err)?;
    let document = ctx.store.document(id).map_err(err)?;
    let extraction = ctx.store.latest_extraction(id).map_err(err)?;
    Ok(json!({
        "document": to_value(&document)?,
        "latest_extraction": to_value(&extraction)?,
    }))
}

fn gst_return(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let date = match args.get("date").and_then(Value::as_str) {
        Some(s) => parse_date(s)?,
        None => Utc::now().date_naive(),
    };
    // The frequency table lives in the rule file for the year the date falls
    // in; the period it yields may end in the next tax year, whose rules then
    // govern the return itself.
    let rules = ctx.rules_for(TaxYear::containing(date))?;
    let frequency = match args.get("frequency").and_then(Value::as_str) {
        Some(id) => rules
            .gst
            .frequency(id)
            .ok_or_else(|| format!("unknown filing frequency {id}"))?,
        None => rules.gst.default_frequency(),
    }
    .frequency()
    .map_err(err)?;
    let period = frequency.period_containing(date);

    let period_year = TaxYear::containing(period.end);
    let rules = if period_year == TaxYear::containing(date) {
        rules
    } else {
        ctx.rules_for(period_year)?
    };
    to_value(&taxreturn::gst101(&ctx.store, &rules, period).map_err(err)?)
}

fn ingest_document(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let path = PathBuf::from(required_str(args, "path")?);
    let bytes = std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let original_filename = args
        .get("original_filename")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()));
    let mime = match args.get("mime").and_then(Value::as_str) {
        Some(m) => m.to_string(),
        None => mime_from_path(&path),
    };
    let source = match args.get("source").and_then(Value::as_str) {
        None | Some("upload") => DocumentSource::Upload,
        Some("photo") => DocumentSource::Photo,
        Some(other) => return Err(format!("unknown source {other}")),
    };

    let intake = ingest(
        &mut ctx.store,
        &ctx.data_dir.clone(),
        IncomingFile {
            bytes,
            mime,
            source,
            original_filename,
        },
    )
    .map_err(err)?;
    let duplicate = matches!(intake, taxingest::Intake::Duplicate(_));
    Ok(json!({
        "duplicate": duplicate,
        "document": to_value(intake.document())?,
    }))
}

fn record_reading_tool(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let document: DocumentId = required_str(args, "document_id")?.parse().map_err(err)?;
    let invoice: ExtractedInvoice = serde_json::from_value(
        args.get("invoice")
            .cloned()
            .ok_or("missing required argument invoice")?,
    )
    .map_err(|e| format!("invoice does not match the documented shape: {e}"))?;

    let today = Utc::now().date_naive();
    let rules = ctx.rules_for(TaxYear::containing(invoice.invoice_date.unwrap_or(today)))?;
    let recorded = record_reading(
        &mut ctx.store,
        document,
        Reading {
            extracted_by: required_str(args, "extracted_by")?.to_string(),
            payload: invoice,
            reported_confidence: args
                .get("confidence")
                .and_then(Value::as_f64)
                .map(|c| c as f32),
        },
        &rules,
        today,
        DEFAULT_CONFIDENCE_FLOOR,
    )
    .map_err(err)?;

    Ok(json!({
        "extraction_id": recorded.extraction.id.to_string(),
        "version": recorded.extraction.version,
        "issues": to_value(&recorded.extraction.issues)?,
        "document_status": to_value(&recorded.document_status)?,
    }))
}

fn propose_draft_tool(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let document: DocumentId = required_str(args, "document_id")?.parse().map_err(err)?;
    let invoice_date = ctx
        .store
        .latest_extraction(document)
        .map_err(err)?
        .and_then(|e| e.payload.invoice_date)
        .unwrap_or_else(|| Utc::now().date_naive());
    let rules = ctx.rules_for(TaxYear::containing(invoice_date))?;

    let entry = propose_draft(
        &mut ctx.store,
        document,
        DraftProposal {
            expense_account: parse_account(required_str(args, "expense_account")?)?,
            funding_account: parse_account(required_str(args, "funding_account")?)?,
            proposed_by: EntrySource::Agent {
                model: required_str(args, "model")?.to_string(),
            },
        },
        &rules,
    )
    .map_err(err)?;

    Ok(json!({
        "note": "draft created; it will not affect any return until a human approves it",
        "entry": to_value(&entry)?,
    }))
}

fn import_bank_rows(ctx: &mut Context, args: &Value) -> Result<Value, String> {
    let account = required_str(args, "account")?.to_string();
    let batch = required_str(args, "batch")?.to_string();
    let rows = args
        .get("rows")
        .and_then(Value::as_array)
        .ok_or("missing required argument rows")?;

    let mut txns = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let date = parse_date(required_str(row, "date").map_err(|e| format!("rows[{i}]: {e}"))?)?;
        let cents = row
            .get("amount_cents")
            .and_then(Value::as_i64)
            .ok_or(format!("rows[{i}]: amount_cents must be an integer"))?;
        let description = required_str(row, "description").map_err(|e| format!("rows[{i}]: {e}"))?;
        let mut txn =
            BankTransaction::new(account.clone(), date, Money::nzd(cents), description, &batch);
        txn.reference = row
            .get("reference")
            .and_then(Value::as_str)
            .map(str::to_string);
        txns.push(txn);
    }

    let inserted = ctx.store.import_bank_txns(&txns).map_err(err)?;
    Ok(json!({
        "received": txns.len(),
        "imported": inserted,
        "skipped_as_duplicates": txns.len() - inserted,
    }))
}

fn parse_document_status(s: &str) -> Result<DocumentStatus, String> {
    match s {
        "pending_extraction" => Ok(DocumentStatus::PendingExtraction),
        "extracted" => Ok(DocumentStatus::Extracted),
        "needs_review" => Ok(DocumentStatus::NeedsReview),
        "ignored" => Ok(DocumentStatus::Ignored),
        other => Err(format!("unknown document status {other}")),
    }
}

fn parse_account(s: &str) -> Result<AccountCode, String> {
    AccountCode::new(s).map_err(err)
}

fn parse_date(s: &str) -> Result<NaiveDate, String> {
    s.parse().map_err(|_| format!("{s} is not a YYYY-MM-DD date"))
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required argument {key}"))
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(err)
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn mime_from_path(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf") => "application/pdf",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("heic") => "image/heic",
        Some("csv") => "text/csv",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use taxcore::{Account, AccountKind, GstTreatment};

    use super::*;

    fn context() -> (Context, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_in_memory().unwrap();
        for account in [
            Account::new(AccountCode::new("6100-office").unwrap(), "Office", AccountKind::Expense)
                .with_gst(GstTreatment::Standard),
            Account::new(AccountCode::new("1010-bank").unwrap(), "Bank", AccountKind::Asset),
        ] {
            store.upsert_account(&account).unwrap();
        }
        let ctx = Context {
            store,
            data_dir: dir.path().to_path_buf(),
            rules_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules"),
        };
        (ctx, dir)
    }

    fn invoice_json() -> Value {
        json!({
            "supplier_name": "Officeworks",
            "supplier_gst_number": "123-456-789",
            "invoice_number": "INV-8842",
            "invoice_date": "2026-05-02",
            "currency": "NZD",
            "subtotal": {"cents": 10000, "currency": "NZD"},
            "gst": {"cents": 1500, "currency": "NZD"},
            "total": {"cents": 11500, "currency": "NZD"},
            "lines": [{"description": "A4 paper", "quantity": "2", "amount": {"cents": 10000, "currency": "NZD"}}],
        })
    }

    #[test]
    fn the_whole_pipeline_runs_over_the_tool_boundary() {
        let (mut ctx, dir) = context();

        // Ingest a file from disk.
        let file = dir.path().join("INV-8842.pdf");
        std::fs::write(&file, b"fake pdf bytes").unwrap();
        let ingested = dispatch(
            &mut ctx,
            "ingest_document",
            &json!({"path": file.to_str().unwrap()}),
        )
        .unwrap();
        assert_eq!(ingested["duplicate"], false);
        let doc_id = ingested["document"]["id"].as_str().unwrap().to_string();

        // Submit a reading; it validates cleanly.
        let recorded = dispatch(
            &mut ctx,
            "record_reading",
            &json!({"document_id": doc_id, "extracted_by": "test-model",
                    "confidence": 0.95, "invoice": invoice_json()}),
        )
        .unwrap();
        assert_eq!(recorded["document_status"], "extracted");
        assert_eq!(recorded["issues"].as_array().unwrap().len(), 0);

        // Propose the draft; it lands in the review queue, not the ledger.
        let drafted = dispatch(
            &mut ctx,
            "propose_draft",
            &json!({"document_id": doc_id, "expense_account": "6100-office",
                    "funding_account": "1010-bank", "model": "test-model"}),
        )
        .unwrap();
        assert_eq!(drafted["entry"]["status"], "draft");

        let queue = dispatch(&mut ctx, "review_queue", &json!({})).unwrap();
        assert_eq!(queue["draft_entries"].as_array().unwrap().len(), 1);

        // The GST return for that period sees nothing: the draft is unconfirmed.
        let gst = dispatch(&mut ctx, "gst_return", &json!({"date": "2026-05-02"})).unwrap();
        assert_eq!(gst["lines"][10]["amount"]["cents"], 0);
    }

    #[test]
    fn bank_rows_import_idempotently_over_the_tool_boundary() {
        let (mut ctx, _dir) = context();
        let rows = json!({"account": "asb-8842", "batch": "jul",
            "rows": [
                {"date": "2026-05-02", "amount_cents": -11500, "description": "OFFICEWORKS"},
                {"date": "2026-05-09", "amount_cents": -4999, "description": "CAFE"},
            ]});
        let first = dispatch(&mut ctx, "import_bank_rows", &rows).unwrap();
        assert_eq!(first["imported"], 2);
        let second = dispatch(&mut ctx, "import_bank_rows", &rows).unwrap();
        assert_eq!(second["imported"], 0);
        assert_eq!(second["skipped_as_duplicates"], 2);

        let unrec = dispatch(&mut ctx, "unreconciled_bank", &json!({})).unwrap();
        assert_eq!(unrec.as_array().unwrap().len(), 2);
    }
}
