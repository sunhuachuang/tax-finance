use taxcore::{
    AccountCode, Currency, DocumentId, DocumentStatus, Entry, EntryBuilder, EntrySource,
    GstTreatment, Posting, Provenance, Rounding, SourceRef,
};
use taxrules::RuleSet;
use taxstore::Store;

use crate::error::{IngestError, not_ready};
use crate::Result;

/// The classification decision an extraction cannot make for itself: which
/// accounts the money moved between. Proposed by an agent or a human — that
/// distinction travels on `proposed_by` and ends up on the entry.
pub struct DraftProposal {
    /// Where the spend lands, e.g. `6100-office`. Its default GST treatment
    /// decides how the amount is treated.
    pub expense_account: AccountCode,
    /// What paid for it, e.g. `1010-bank` or an owner-funds account.
    pub funding_account: AccountCode,
    pub proposed_by: EntrySource,
}

/// Turn a cleanly-extracted purchase document into a *draft* ledger entry.
///
/// The model reported the figures; this function does the only computation —
/// the GST split, from the rule file — and builds the double entry. The result
/// is always a draft with document + extraction provenance: there is no
/// argument that yields a posted entry.
pub fn propose_draft(
    store: &mut Store,
    document: DocumentId,
    proposal: DraftProposal,
    rules: &RuleSet,
) -> Result<Entry> {
    let doc = store.document(document)?;
    if doc.status != DocumentStatus::Extracted {
        return Err(not_ready(
            document,
            format!("its status is {:?}, drafting needs a clean extraction", doc.status),
        ));
    }
    let extraction = store
        .latest_extraction(document)?
        .ok_or_else(|| not_ready(document, "it has no extraction on file"))?;
    if extraction.has_errors() {
        // Unreachable through record_reading, which demotes to NeedsReview;
        // checked anyway so a status set by hand cannot smuggle bad numbers in.
        return Err(not_ready(document, "its latest extraction failed validation"));
    }

    let invoice = &extraction.payload;
    if invoice.currency != Currency::NZD {
        return Err(IngestError::UnsupportedCurrency(
            invoice.currency.to_string(),
        ));
    }
    let date = invoice
        .invoice_date
        .ok_or_else(|| not_ready(document, "the extraction has no invoice date"))?;

    let expense = store.account(&proposal.expense_account)?;
    let funding = store.account(&proposal.funding_account)?;
    for account in [&expense, &funding] {
        if !account.active {
            return Err(IngestError::RetiredAccount(account.code.to_string()));
        }
    }

    // A default is a starting point for review, never a silent decision —
    // which is exactly what a draft entry is.
    let treatment = expense
        .default_gst_treatment
        .unwrap_or(GstTreatment::Standard);
    let gst = if treatment.attracts_gst() {
        Some(match invoice.gst {
            // The stated figure already survived validation; keeping it means
            // the ledger reconciles cent-for-cent with the document.
            Some(stated) if !stated.is_zero() => stated,
            _ => rules
                .gst_rate()
                .extract_from_inclusive(invoice.total, Rounding::HalfUp)?,
        })
    } else {
        None
    };

    let supplier = invoice.supplier_name.as_deref().unwrap_or("Unknown supplier");
    let narration = match &invoice.invoice_number {
        Some(number) => format!("{supplier} {number}"),
        None => supplier.to_string(),
    };

    let mut debit = Posting::new(expense.code.clone(), invoice.total.abs(), treatment);
    if let Some(gst) = gst {
        debit = debit.with_gst_amount(gst);
    }
    let entry = EntryBuilder::new(date, narration, proposal.proposed_by)
        .posting(debit)
        .credit(funding.code.clone(), invoice.total, GstTreatment::NotSubject)
        .build()?;

    let provenance = Provenance::new(
        entry.id,
        vec![
            SourceRef::Document(document),
            SourceRef::Extraction(extraction.id),
        ],
    );
    store.insert_entry(&entry, &provenance)?;
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use taxcore::{EntryStatus, Money};

    use crate::testutil::{clean_invoice, code, rules, store, today, upload};
    use crate::{DEFAULT_CONFIDENCE_FLOOR, Intake, Reading, ingest, record_reading};

    use super::*;

    fn proposal() -> DraftProposal {
        DraftProposal {
            expense_account: code("6100-office"),
            funding_account: code("1010-bank"),
            proposed_by: EntrySource::Agent {
                model: "test-model".into(),
            },
        }
    }

    fn extracted_document(store: &mut Store, rules: &RuleSet) -> DocumentId {
        let dir = tempfile::tempdir().unwrap();
        let Intake::Stored(doc) = ingest(store, dir.path(), upload(b"bytes")).unwrap() else {
            panic!("fresh store");
        };
        record_reading(
            store,
            doc.id,
            Reading {
                extracted_by: "test-model".into(),
                payload: clean_invoice(),
                reported_confidence: Some(0.95),
            },
            rules,
            today(),
            DEFAULT_CONFIDENCE_FLOOR,
        )
        .unwrap();
        doc.id
    }

    #[test]
    fn a_clean_extraction_drafts_a_balanced_entry_with_provenance() {
        let mut store = store();
        let rules = rules();
        let doc = extracted_document(&mut store, &rules);

        let entry = propose_draft(&mut store, doc, proposal(), &rules).unwrap();

        assert_eq!(entry.status, EntryStatus::Draft);
        assert!(entry.is_balanced());
        assert_eq!(entry.narration, "Officeworks INV-8842");
        assert_eq!(entry.total_gst().unwrap(), Money::nzd(1500));
        assert_eq!(entry.postings[0].amount, Money::nzd(11500));
        assert_eq!(entry.postings[1].amount, Money::nzd(-11500));

        let provenance = store.provenance(entry.id).unwrap();
        assert!(provenance.is_backed_by_document());
        assert_eq!(provenance.sources.len(), 2);
    }

    #[test]
    fn a_document_in_review_cannot_be_drafted() {
        let mut store = store();
        let rules = rules();
        let doc = extracted_document(&mut store, &rules);
        store
            .set_document_status(doc, taxcore::DocumentStatus::NeedsReview)
            .unwrap();

        let err = propose_draft(&mut store, doc, proposal(), &rules).unwrap_err();
        assert!(matches!(err, IngestError::NotReady { .. }));
    }

    #[test]
    fn a_missing_account_is_an_error_not_a_guess() {
        let mut store = store();
        let rules = rules();
        let doc = extracted_document(&mut store, &rules);

        let bad = DraftProposal {
            expense_account: code("9999-nope"),
            ..proposal()
        };
        let err = propose_draft(&mut store, doc, bad, &rules).unwrap_err();
        assert!(matches!(
            err,
            IngestError::Store(taxstore::StoreError::NotFound { .. })
        ));
    }

    #[test]
    fn foreign_invoices_are_refused_for_now() {
        let mut store = store();
        let rules = rules();

        let dir = tempfile::tempdir().unwrap();
        let Intake::Stored(doc) = ingest(&mut store, dir.path(), upload(b"usd bytes")).unwrap()
        else {
            panic!("fresh store");
        };
        let mut invoice = clean_invoice();
        invoice.currency = taxcore::Currency::USD;
        invoice.subtotal = None;
        invoice.gst = None;
        invoice.total = Money::new(11500, taxcore::Currency::USD);
        invoice.lines[0].amount = Money::new(11500, taxcore::Currency::USD);
        record_reading(
            &mut store,
            doc.id,
            Reading {
                extracted_by: "test-model".into(),
                payload: invoice,
                reported_confidence: Some(0.95),
            },
            &rules,
            today(),
            DEFAULT_CONFIDENCE_FLOOR,
        )
        .unwrap();

        let err = propose_draft(&mut store, doc.id, proposal(), &rules).unwrap_err();
        assert!(matches!(err, IngestError::UnsupportedCurrency(_)));
    }
}
