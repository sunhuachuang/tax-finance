//! `--demo`: seed a throwaway ledger so every section of the page has
//! something to show, including one draft waiting for the approve button.

use std::path::Path;

use chrono::NaiveDate;
use taxcore::{
    Account, AccountCode, AccountKind, BankTransaction, Currency, DocumentSource, EntryBuilder,
    EntrySource, ExtractedInvoice, GstTreatment, LineItem, Money, Posting, Provenance, SourceRef,
    TaxYear,
};
use taxingest::{
    DEFAULT_CONFIDENCE_FLOOR, DraftProposal, IncomingFile, Intake, Reading, ingest, propose_draft,
    record_reading,
};
use taxrules::RuleSet;
use taxstore::Store;

pub fn seed(store: &mut Store, data_dir: &Path, rules_dir: &Path) -> anyhow::Result<()> {
    let rules = RuleSet::for_year(rules_dir, "NZ", TaxYear(2026))?;
    let code = |s: &str| AccountCode::new(s).unwrap();

    for account in [
        Account::new(code("4000-sales"), "Sales", AccountKind::Income)
            .with_gst(GstTreatment::Standard),
        Account::new(code("4100-exports"), "Export sales", AccountKind::Income)
            .with_gst(GstTreatment::ZeroRated),
        Account::new(code("6100-office"), "Office supplies", AccountKind::Expense)
            .with_gst(GstTreatment::Standard),
        Account::new(code("6200-wages"), "Wages", AccountKind::Expense)
            .with_gst(GstTreatment::NotSubject),
        Account::new(code("1010-bank"), "Bank", AccountKind::Asset),
    ] {
        store.upsert_account(&account)?;
    }

    // Posted history inside the 2025-26 year.
    let d = |m: u32, day: u32| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
    for entry in [
        EntryBuilder::new(d(2, 10), "Consulting invoice #41", EntrySource::Human)
            .posting(
                Posting::new(code("4000-sales"), Money::nzd(-23000), GstTreatment::Standard)
                    .with_gst_amount(Money::nzd(-3000)),
            )
            .debit(code("1010-bank"), Money::nzd(23000), GstTreatment::NotSubject)
            .build()?,
        EntryBuilder::new(d(2, 12), "Export invoice #42", EntrySource::Human)
            .credit(code("4100-exports"), Money::nzd(50000), GstTreatment::ZeroRated)
            .debit(code("1010-bank"), Money::nzd(50000), GstTreatment::NotSubject)
            .build()?,
        EntryBuilder::new(d(2, 20), "Stationery", EntrySource::Human)
            .posting(
                Posting::new(code("6100-office"), Money::nzd(6900), GstTreatment::Standard)
                    .with_gst_amount(Money::nzd(900)),
            )
            .credit(code("1010-bank"), Money::nzd(6900), GstTreatment::NotSubject)
            .build()?,
        EntryBuilder::new(d(3, 1), "Wages", EntrySource::Human)
            .debit(code("6200-wages"), Money::nzd(20000), GstTreatment::NotSubject)
            .credit(code("1010-bank"), Money::nzd(20000), GstTreatment::NotSubject)
            .build()?,
    ] {
        let id = entry.id;
        store.insert_entry(&entry, &Provenance::new(id, vec![SourceRef::Manual]))?;
        store.post_entry(id)?;
    }

    // A clean extraction drafted through the pipeline — waiting for approval.
    let Intake::Stored(doc) = ingest(
        store,
        data_dir,
        IncomingFile {
            bytes: b"demo: officeworks invoice INV-8842".to_vec(),
            mime: "application/pdf".into(),
            source: DocumentSource::Upload,
            original_filename: Some("INV-8842.pdf".into()),
        },
    )?
    else {
        anyhow::bail!("demo store is fresh; ingest cannot dedup");
    };
    record_reading(
        store,
        doc.id,
        Reading {
            extracted_by: "demo-model".into(),
            payload: invoice("Officeworks", "INV-8842", d(2, 14), 11500, 1500),
            reported_confidence: Some(0.95),
        },
        &rules,
        d(2, 15),
        DEFAULT_CONFIDENCE_FLOOR,
    )?;
    propose_draft(
        store,
        doc.id,
        DraftProposal {
            expense_account: code("6100-office"),
            funding_account: code("1010-bank"),
            proposed_by: EntrySource::Agent {
                model: "demo-model".into(),
            },
        },
        &rules,
    )?;

    // A blurry photo the model was unsure about — lands in review.
    let Intake::Stored(blurry) = ingest(
        store,
        data_dir,
        IncomingFile {
            bytes: b"demo: blurry cafe receipt".to_vec(),
            mime: "image/jpeg".into(),
            source: DocumentSource::Photo,
            original_filename: None,
        },
    )?
    else {
        anyhow::bail!("demo store is fresh; ingest cannot dedup");
    };
    record_reading(
        store,
        blurry.id,
        Reading {
            extracted_by: "demo-model".into(),
            payload: invoice("Espresso Corner", "0417", d(2, 18), 4999, 652),
            reported_confidence: Some(0.35),
        },
        &rules,
        d(2, 19),
        DEFAULT_CONFIDENCE_FLOOR,
    )?;

    // Bank rows: one matches the drafted invoice, one is unexplained.
    store.import_bank_txns(&[
        BankTransaction::new(
            "asb-8842",
            d(2, 14),
            Money::nzd(-11500),
            "OFFICEWORKS AUCKLAND",
            "demo-feb",
        ),
        BankTransaction::new(
            "asb-8842",
            d(2, 18),
            Money::nzd(-4999),
            "ESPRESSO CORNER",
            "demo-feb",
        ),
        BankTransaction::new(
            "asb-8842",
            d(2, 10),
            Money::nzd(23000),
            "DEPOSIT CONSULTING",
            "demo-feb",
        ),
    ])?;

    Ok(())
}

fn invoice(
    supplier: &str,
    number: &str,
    date: NaiveDate,
    total: i64,
    gst: i64,
) -> ExtractedInvoice {
    ExtractedInvoice {
        supplier_name: Some(supplier.into()),
        supplier_gst_number: Some("123-456-789".into()),
        invoice_number: Some(number.into()),
        invoice_date: Some(date),
        currency: Currency::NZD,
        subtotal: Some(Money::nzd(total - gst)),
        gst: Some(Money::nzd(gst)),
        total: Money::nzd(total),
        lines: vec![LineItem {
            description: "as per invoice".into(),
            quantity: None,
            amount: Money::nzd(total - gst),
        }],
    }
}
