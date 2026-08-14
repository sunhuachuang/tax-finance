//! The ingestion pipeline: upload → extraction → validation → review → ledger.
//!
//! This crate is the orchestrator taxcore deliberately refuses to be. It does
//! the I/O — files on disk, rows in the store — and drives the state machines,
//! but every judgement stays where the architecture put it: agents *report*
//! fields ([`record_reading`]), arithmetic decides whether to believe them
//! (`ExtractedInvoice::validate`), deterministic code computes the numbers
//! that reach the ledger ([`propose_draft`]), and a human owns confirmation
//! ([`review`]).
//!
//! The trust model, end to end:
//!
//! 1. [`ingest`] stores bytes content-addressed and dedups by hash; the
//!    document enters as `PendingExtraction`.
//! 2. An agent reads the bytes and submits a [`Reading`]; [`record_reading`]
//!    versions it, validates it arithmetically, and moves the document to
//!    `Extracted` or `NeedsReview`. A stated confidence can only demote,
//!    never promote.
//! 3. [`propose_draft`] turns a clean extraction into a *draft* entry with
//!    full provenance. GST content is computed from the rule file, not taken
//!    from the model. There is no path from extraction to posted entry that
//!    skips the draft state.
//! 4. [`review_queue`] / [`approve_draft`] / [`reject_draft`] are the human
//!    gate. Rejection voids — nothing is deleted.

mod draft;
mod error;
mod intake;
mod reading;
mod reconcile;
mod review;

pub use draft::{DraftProposal, propose_draft};
pub use error::{IngestError, Result};
pub use intake::{IncomingFile, Intake, document_bytes, ingest};
pub use reading::{Reading, RecordedReading, record_reading};
pub use reconcile::propose_matches;
pub use review::{ReviewQueue, approve_draft, reject_draft, review_queue};

/// Below this reported confidence an extraction goes to review even when its
/// arithmetic is clean.
pub const DEFAULT_CONFIDENCE_FLOOR: f32 = 0.8;

#[cfg(test)]
mod testutil {
    use std::path::PathBuf;

    use chrono::NaiveDate;
    use taxcore::{
        Account, AccountCode, AccountKind, Currency, DocumentSource, ExtractedInvoice,
        GstTreatment, LineItem, Money, TaxYear,
    };
    use taxrules::RuleSet;
    use taxstore::Store;

    pub fn store() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        store
            .upsert_account(
                &Account::new(code("6100-office"), "Office supplies", AccountKind::Expense)
                    .with_gst(GstTreatment::Standard),
            )
            .unwrap();
        store
            .upsert_account(&Account::new(code("1010-bank"), "Bank", AccountKind::Asset))
            .unwrap();
        store
    }

    pub fn rules() -> RuleSet {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules");
        RuleSet::for_year(&dir, "NZ", TaxYear(2026)).unwrap()
    }

    pub fn code(s: &str) -> AccountCode {
        AccountCode::new(s).unwrap()
    }

    pub fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 31).unwrap()
    }

    pub fn upload(bytes: &[u8]) -> crate::IncomingFile {
        crate::IncomingFile {
            bytes: bytes.to_vec(),
            mime: "application/pdf".into(),
            source: DocumentSource::Upload,
            original_filename: Some("INV-8842.pdf".into()),
        }
    }

    pub fn clean_invoice() -> ExtractedInvoice {
        ExtractedInvoice {
            supplier_name: Some("Officeworks".into()),
            supplier_gst_number: Some("123-456-789".into()),
            invoice_number: Some("INV-8842".into()),
            invoice_date: Some(NaiveDate::from_ymd_opt(2026, 5, 2).unwrap()),
            currency: Currency::NZD,
            subtotal: Some(Money::nzd(10000)),
            gst: Some(Money::nzd(1500)),
            total: Money::nzd(11500),
            lines: vec![LineItem {
                description: "A4 paper".into(),
                quantity: Some("2".into()),
                amount: Money::nzd(10000),
            }],
        }
    }
}
