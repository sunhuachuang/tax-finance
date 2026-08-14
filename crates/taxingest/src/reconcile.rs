use std::collections::HashSet;

use taxcore::bank::DEFAULT_MATCH_WINDOW_DAYS;
use taxcore::{DocumentId, DocumentStatus, MatchCandidate, MatchStrength};
use taxstore::Store;

use crate::Result;

/// Propose which extracted documents explain which bank rows. The engine only
/// proposes — confirming a match (and drafting the entry it implies) stays
/// with the reviewer.
///
/// Money-out rows are compared against unmatched extracted purchases on exact
/// amount within a date window: same-day with the supplier visible in the bank
/// description is [`MatchStrength::Exact`], inside the window is `Probable`,
/// at its edge is `Possible`. Results come back strongest first.
pub fn propose_matches(store: &Store, window_days: Option<i64>) -> Result<Vec<MatchCandidate>> {
    let window = window_days.unwrap_or(DEFAULT_MATCH_WINDOW_DAYS);
    let claimed: HashSet<DocumentId> = store.matched_document_ids()?.into_iter().collect();

    // (document, invoice) pairs still looking for a payment.
    let mut invoices = Vec::new();
    for doc in store.documents_with_status(DocumentStatus::Extracted)? {
        if claimed.contains(&doc.id) {
            continue;
        }
        if let Some(extraction) = store.latest_extraction(doc.id)? {
            invoices.push((doc.id, extraction.payload));
        }
    }

    let mut candidates = Vec::new();
    for txn in store.unreconciled_bank_txns()? {
        if txn.matched_document.is_some() || !txn.is_money_out() {
            continue;
        }
        let description = txn.description.to_lowercase();

        for (document, invoice) in &invoices {
            if invoice.total != txn.amount.abs() {
                continue;
            }
            let Some(invoice_date) = invoice.invoice_date else {
                continue;
            };
            let day_gap = (txn.date - invoice_date).num_days().abs();
            if day_gap > window {
                continue;
            }

            let supplier_seen = invoice
                .supplier_name
                .as_deref()
                .map(|name| supplier_in_description(name, &description))
                .unwrap_or(false);

            let strength = if day_gap == 0 && supplier_seen {
                MatchStrength::Exact
            } else if day_gap < window {
                MatchStrength::Probable
            } else {
                MatchStrength::Possible
            };
            let reason = format!(
                "amount {} matches, {} day(s) apart{}",
                invoice.total,
                day_gap,
                if supplier_seen {
                    ", supplier seen in description"
                } else {
                    ""
                }
            );
            candidates.push(MatchCandidate {
                bank_txn: txn.id,
                document: *document,
                strength,
                day_gap,
                reason,
            });
        }
    }

    candidates.sort_by(|a, b| a.strength.cmp(&b.strength).then(a.day_gap.cmp(&b.day_gap)));
    Ok(candidates)
}

/// Any word of the supplier name (3+ chars, so "the"-like noise still counts
/// but single letters never do) appearing in the bank description.
fn supplier_in_description(supplier: &str, description_lower: &str) -> bool {
    supplier
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.len() >= 3)
        .any(|word| description_lower.contains(word))
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use taxcore::{BankTransaction, Money};

    use crate::testutil::{clean_invoice, rules, store, today, upload};
    use crate::{DEFAULT_CONFIDENCE_FLOOR, Intake, Reading, ingest, record_reading};

    use super::*;

    fn extracted_doc(store: &mut Store, bytes: &[u8]) -> DocumentId {
        let dir = tempfile::tempdir().unwrap();
        let Intake::Stored(doc) = ingest(store, dir.path(), upload(bytes)).unwrap() else {
            panic!()
        };
        record_reading(
            store,
            doc.id,
            Reading {
                extracted_by: "test-model".into(),
                payload: clean_invoice(), // total 115.00, dated 2026-05-02, Officeworks
                reported_confidence: Some(0.95),
            },
            &rules(),
            today(),
            DEFAULT_CONFIDENCE_FLOOR,
        )
        .unwrap();
        doc.id
    }

    fn payment(date: NaiveDate, cents: i64, description: &str) -> BankTransaction {
        BankTransaction::new("asb-8842", date, Money::nzd(cents), description, "batch")
    }

    #[test]
    fn strength_tracks_date_and_supplier() {
        let mut store = store();
        let doc = extracted_doc(&mut store, b"invoice");
        let invoice_date = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();

        store
            .import_bank_txns(&[
                payment(invoice_date, -11500, "OFFICEWORKS AUCKLAND"),
                payment(invoice_date + chrono::Days::new(2), -11500, "POS W/D 4421"),
                payment(invoice_date + chrono::Days::new(3), -11500, "POS W/D 4422"),
                payment(invoice_date, -99999, "SOMETHING ELSE"),
                payment(invoice_date + chrono::Days::new(30), -11500, "TOO LATE"),
            ])
            .unwrap();

        let candidates = propose_matches(&store, None).unwrap();
        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].strength, MatchStrength::Exact);
        assert_eq!(candidates[0].document, doc);
        assert_eq!(candidates[1].strength, MatchStrength::Probable);
        assert_eq!(candidates[1].day_gap, 2);
        assert_eq!(candidates[2].strength, MatchStrength::Possible);
        assert_eq!(candidates[2].day_gap, 3);
    }

    #[test]
    fn claimed_documents_and_money_in_are_skipped() {
        let mut store = store();
        let doc = extracted_doc(&mut store, b"invoice");
        let invoice_date = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();

        let paid = payment(invoice_date, -11500, "OFFICEWORKS");
        let refund = payment(invoice_date, 11500, "OFFICEWORKS REFUND");
        store
            .import_bank_txns(&[paid.clone(), refund])
            .unwrap();

        assert_eq!(propose_matches(&store, None).unwrap().len(), 1);

        store.link_bank_txn(paid.id, Some(doc), None).unwrap();
        assert!(propose_matches(&store, None).unwrap().is_empty());
    }
}
