use taxcore::{Document, DocumentStatus, Entry, EntryId, EntryStatus};
use taxstore::Store;

use crate::Result;

/// Everything waiting on a human: documents whose extraction failed or was
/// unsure, and draft entries not yet confirmed.
pub struct ReviewQueue {
    pub documents: Vec<Document>,
    pub drafts: Vec<Entry>,
}

impl ReviewQueue {
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty() && self.drafts.is_empty()
    }
}

pub fn review_queue(store: &Store) -> Result<ReviewQueue> {
    Ok(ReviewQueue {
        documents: store.documents_with_status(DocumentStatus::NeedsReview)?,
        drafts: store.entries_with_status(EntryStatus::Draft)?,
    })
}

/// The confirmation that makes an entry real. This is the write the MCP layer
/// must never expose to an agent.
pub fn approve_draft(store: &mut Store, entry: EntryId) -> Result<()> {
    Ok(store.post_entry(entry)?)
}

/// Reject a proposal. The draft is voided in place — visible forever as
/// something that was suggested and turned down.
pub fn reject_draft(store: &mut Store, entry: EntryId) -> Result<()> {
    Ok(store.void_entry(entry)?)
}

#[cfg(test)]
mod tests {
    use taxcore::EntrySource;

    use crate::testutil::{clean_invoice, code, rules, store, today, upload};
    use crate::{DEFAULT_CONFIDENCE_FLOOR, DraftProposal, Intake, Reading, ingest, propose_draft, record_reading};

    use super::*;

    #[test]
    fn the_queue_fills_and_drains_through_the_two_gates() {
        let mut store = store();
        let rules = rules();
        let dir = tempfile::tempdir().unwrap();

        // One document extracts cleanly and gets drafted; one lands in review.
        let Intake::Stored(clean) = ingest(&mut store, dir.path(), upload(b"clean")).unwrap()
        else {
            panic!()
        };
        let Intake::Stored(unsure) = ingest(&mut store, dir.path(), upload(b"unsure")).unwrap()
        else {
            panic!()
        };
        for (doc, confidence) in [(&clean, 0.95), (&unsure, 0.2)] {
            record_reading(
                &mut store,
                doc.id,
                Reading {
                    extracted_by: "test-model".into(),
                    payload: clean_invoice(),
                    reported_confidence: Some(confidence),
                },
                &rules,
                today(),
                DEFAULT_CONFIDENCE_FLOOR,
            )
            .unwrap();
        }
        let draft = propose_draft(
            &mut store,
            clean.id,
            DraftProposal {
                expense_account: code("6100-office"),
                funding_account: code("1010-bank"),
                proposed_by: EntrySource::Agent {
                    model: "test-model".into(),
                },
            },
            &rules,
        )
        .unwrap();

        let queue = review_queue(&store).unwrap();
        assert_eq!(queue.documents.len(), 1);
        assert_eq!(queue.documents[0].id, unsure.id);
        assert_eq!(queue.drafts.len(), 1);
        assert_eq!(queue.drafts[0].id, draft.id);

        approve_draft(&mut store, draft.id).unwrap();
        assert_eq!(store.entry(draft.id).unwrap().status, EntryStatus::Posted);

        store
            .set_document_status(unsure.id, DocumentStatus::Ignored)
            .unwrap();
        assert!(review_queue(&store).unwrap().is_empty());
    }

    #[test]
    fn rejection_voids_and_the_entry_never_posts() {
        let mut store = store();
        let rules = rules();
        let dir = tempfile::tempdir().unwrap();

        let Intake::Stored(doc) = ingest(&mut store, dir.path(), upload(b"bytes")).unwrap()
        else {
            panic!()
        };
        record_reading(
            &mut store,
            doc.id,
            Reading {
                extracted_by: "test-model".into(),
                payload: clean_invoice(),
                reported_confidence: Some(0.95),
            },
            &rules,
            today(),
            DEFAULT_CONFIDENCE_FLOOR,
        )
        .unwrap();
        let draft = propose_draft(
            &mut store,
            doc.id,
            DraftProposal {
                expense_account: code("6100-office"),
                funding_account: code("1010-bank"),
                proposed_by: EntrySource::Human,
            },
            &rules,
        )
        .unwrap();

        reject_draft(&mut store, draft.id).unwrap();
        assert_eq!(store.entry(draft.id).unwrap().status, EntryStatus::Voided);
        assert!(approve_draft(&mut store, draft.id).is_err());
    }
}
