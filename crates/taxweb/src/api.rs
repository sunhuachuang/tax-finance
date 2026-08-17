//! Routes. Reads mirror what MCP exposes; the two confirmation routes
//! (approve/reject) exist *only* here, behind a click on localhost — this
//! page is the human gate.

use std::path::PathBuf;

use chrono::{NaiveDate, Utc};
use serde_json::{Value, json};
use taxcore::{DocumentId, DocumentSource, DocumentStatus, EntryId, EntryStatus, TaxYear};
use taxingest::{IncomingFile, Intake, propose_matches, review_queue};
use taxrules::RuleSet;
use taxstore::Store;

use crate::http::Request;

pub struct Ctx {
    pub store: Store,
    pub data_dir: PathBuf,
    pub rules_dir: PathBuf,
}

impl Ctx {
    fn rules_for(&self, year: TaxYear) -> Result<RuleSet, String> {
        RuleSet::for_year(&self.rules_dir, "NZ", year).map_err(|e| e.to_string())
    }
}

pub fn route(ctx: &mut Ctx, req: &Request) -> (u16, &'static str, Vec<u8>) {
    let result = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => {
            return (
                200,
                "text/html; charset=utf-8",
                include_bytes!("index.html").to_vec(),
            );
        }
        ("GET", "/api/overview") => overview(ctx),
        ("GET", "/api/gst") => gst(ctx, req),
        ("GET", "/api/ir3") => ir3(ctx, req),
        ("POST", "/api/documents") => ingest(ctx, req),
        ("GET", path) if path.starts_with("/api/documents/") => document(ctx, path),
        ("POST", path) if path.starts_with("/api/documents/") => document_status(ctx, path, req),
        ("POST", path) if path.starts_with("/api/entries/") => entry_action(ctx, path),
        ("GET", _) => return (404, "application/json", br#"{"error":"not found"}"#.to_vec()),
        _ => {
            return (
                405,
                "application/json",
                br#"{"error":"method not allowed"}"#.to_vec(),
            );
        }
    };

    match result {
        Ok(value) => (200, "application/json", value.to_string().into_bytes()),
        Err(message) => (
            400,
            "application/json",
            json!({ "error": message }).to_string().into_bytes(),
        ),
    }
}

fn overview(ctx: &mut Ctx) -> Result<Value, String> {
    let queue = review_queue(&ctx.store).map_err(err)?;
    let mut documents = Vec::new();
    for status in [
        DocumentStatus::PendingExtraction,
        DocumentStatus::Extracted,
        DocumentStatus::NeedsReview,
        DocumentStatus::Ignored,
    ] {
        documents.extend(ctx.store.documents_with_status(status).map_err(err)?);
    }
    Ok(json!({
        "review_documents": to_value(&queue.documents)?,
        "review_drafts": to_value(&queue.drafts)?,
        "documents": to_value(&documents)?,
        "posted_entries": to_value(&ctx.store.entries_with_status(EntryStatus::Posted).map_err(err)?)?,
        "unreconciled_bank": to_value(&ctx.store.unreconciled_bank_txns().map_err(err)?)?,
        "match_candidates": to_value(&propose_matches(&ctx.store, None).map_err(err)?)?,
        "accounts": to_value(&ctx.store.accounts(false).map_err(err)?)?,
    }))
}

fn gst(ctx: &mut Ctx, req: &Request) -> Result<Value, String> {
    let date = match req.query.get("date") {
        Some(s) => s
            .parse::<NaiveDate>()
            .map_err(|_| format!("{s} is not a YYYY-MM-DD date"))?,
        None => Utc::now().date_naive(),
    };
    let rules = ctx.rules_for(TaxYear::containing(date))?;
    let frequency = match req.query.get("frequency").map(String::as_str) {
        None | Some("") => rules.gst.default_frequency(),
        Some(id) => rules
            .gst
            .frequency(id)
            .ok_or_else(|| format!("unknown filing frequency {id}"))?,
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

fn ir3(ctx: &mut Ctx, req: &Request) -> Result<Value, String> {
    let year: TaxYear = req
        .query
        .get("year")
        .ok_or("missing query parameter year")?
        .parse()
        .map_err(err)?;
    let rules = ctx.rules_for(year)?;
    to_value(&taxreturn::ir3(&ctx.store, &rules, year).map_err(err)?)
}

/// POST /api/documents?filename=... — the raw file as the body, its mime in
/// `Content-Type`. Deliberately not multipart: one file per request needs no
/// parser, and this route exists so the phone can send a photo of a receipt to
/// the one ledger rather than keeping a second copy of its own.
///
/// This is a write, but it only creates a `PendingExtraction` document — the
/// same standing as the MCP layer's `ingest_document`. Nothing here becomes a
/// journal entry, let alone a posted one.
fn ingest(ctx: &mut Ctx, req: &Request) -> Result<Value, String> {
    if req.body_too_large {
        return Err("upload exceeds the size limit".to_string());
    }
    if req.body.is_empty() {
        return Err("upload has no body".to_string());
    }

    let file = IncomingFile {
        bytes: req.body.clone(),
        mime: req
            .content_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_string()),
        // Whatever sent this used the upload route; recording it as anything
        // else would put a claim into provenance that nothing supports.
        source: DocumentSource::Upload,
        original_filename: req.query.get("filename").cloned(),
    };

    let intake = taxingest::ingest(&mut ctx.store, &ctx.data_dir, file).map_err(err)?;
    Ok(json!({
        "duplicate": matches!(intake, Intake::Duplicate(_)),
        "document": to_value(intake.document())?,
    }))
}

/// GET /api/documents/{id} — one document with every reading recorded against
/// it, newest version first.
///
/// The extractions are the point. A document's status says *that* something
/// failed validation; the extraction says *what*, which model said it, and how
/// sure it claimed to be. None of that should require opening the database.
fn document(ctx: &mut Ctx, path: &str) -> Result<Value, String> {
    let id: DocumentId = path
        .trim_start_matches("/api/documents/")
        .parse()
        .map_err(err)?;
    let document = ctx.store.document(id).map_err(err)?;
    let mut extractions = ctx.store.extractions_for(id).map_err(err)?;
    extractions.reverse();

    Ok(json!({
        "document": to_value(&document)?,
        "extractions": to_value(&extractions)?,
        // Where the file sits *on this machine*. Useful to a client running
        // here; meaningless to one on a phone, which is why the app's remote
        // backend blanks it rather than offering to open a path it cannot see.
        "local_path": ctx.data_dir.join(&document.stored_path).to_string_lossy(),
    }))
}

/// POST /api/documents/{id}/status?to=ignored — a human decision.
///
/// Only the transitions the store allows: ignoring a document (a duplicate, a
/// personal receipt, spam) and putting an ignored one back in the queue.
/// Nothing here claims a document was *read* — that is `record_reading`, and it
/// belongs to whatever actually read the bytes.
fn document_status(ctx: &mut Ctx, path: &str, req: &Request) -> Result<Value, String> {
    let rest = path.trim_start_matches("/api/documents/");
    let (id, action) = rest
        .split_once('/')
        .ok_or("expected /api/documents/{id}/status")?;
    if action != "status" {
        return Err(format!("unknown action {action}"));
    }
    let id: DocumentId = id.parse().map_err(err)?;

    let to = match req.query.get("to").map(String::as_str) {
        Some("ignored") => DocumentStatus::Ignored,
        Some("pending_extraction") => DocumentStatus::PendingExtraction,
        Some(other) => {
            return Err(format!(
                "status {other} is not a human decision; only ignored and \
                 pending_extraction can be set here"
            ));
        }
        None => return Err("missing query parameter to".to_string()),
    };

    ctx.store.set_document_status(id, to).map_err(err)?;
    Ok(json!({ "ok": true, "status": to_value(&to)? }))
}

/// POST /api/entries/{id}/approve | /api/entries/{id}/reject — the human gate.
fn entry_action(ctx: &mut Ctx, path: &str) -> Result<Value, String> {
    let rest = path.trim_start_matches("/api/entries/");
    let (id, action) = rest
        .split_once('/')
        .ok_or("expected /api/entries/{id}/{approve|reject}")?;
    let id: EntryId = id.parse().map_err(err)?;
    match action {
        "approve" => taxingest::approve_draft(&mut ctx.store, id).map_err(err)?,
        "reject" => taxingest::reject_draft(&mut ctx.store, id).map_err(err)?,
        other => return Err(format!("unknown action {other}")),
    }
    Ok(json!({ "ok": true }))
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(err)
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
