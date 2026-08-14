use chrono::NaiveDate;
use taxcore::{DocumentId, DocumentStatus, ExtractedInvoice, Extraction};
use taxrules::RuleSet;
use taxstore::Store;

use crate::Result;
use crate::error::not_ready;

/// What an agent (or a human typing numbers in) hands back after looking at a
/// document. The pipeline never sees the model call itself.
pub struct Reading {
    /// The model that did the work, or `"human"`.
    pub extracted_by: String,
    pub payload: ExtractedInvoice,
    /// The reader's own confidence. Advisory: it can send a clean extraction
    /// to review, never rescue a broken one.
    pub reported_confidence: Option<f32>,
}

#[derive(Debug)]
pub struct RecordedReading {
    pub extraction: Extraction,
    /// Where the document landed: `Extracted` on the happy path,
    /// `NeedsReview` when arithmetic or confidence said stop.
    pub document_status: DocumentStatus,
}

/// Record one reading of a document and advance the document's state machine.
///
/// Versions are assigned here (latest + 1), the payload is validated
/// arithmetically against the rule file's GST rate, and the resulting status
/// follows the trust model: errors → review, low confidence → review,
/// otherwise extracted. Re-reading a document that is already extracted or in
/// review just adds a version — earlier readings are superseded, not lost.
pub fn record_reading(
    store: &mut Store,
    document: DocumentId,
    reading: Reading,
    rules: &RuleSet,
    today: NaiveDate,
    confidence_floor: f32,
) -> Result<RecordedReading> {
    let doc = store.document(document)?;
    if doc.status == DocumentStatus::Ignored {
        return Err(not_ready(
            document,
            "it is marked ignored; un-ignore it first",
        ));
    }

    let version = store
        .latest_extraction(document)?
        .map(|e| e.version + 1)
        .unwrap_or(1);

    let mut extraction = Extraction::new(
        document,
        version,
        reading.extracted_by,
        reading.payload,
        rules.gst_rate(),
        today,
    );
    if let Some(confidence) = reading.reported_confidence {
        extraction = extraction.with_reported_confidence(confidence);
    }

    store.insert_extraction(&extraction)?;
    let document_status = extraction.resulting_status(confidence_floor);
    store.set_document_status(document, document_status)?;

    Ok(RecordedReading {
        extraction,
        document_status,
    })
}

#[cfg(test)]
mod tests {
    use taxcore::Money;

    use crate::testutil::{clean_invoice, rules, store, today, upload};
    use crate::{DEFAULT_CONFIDENCE_FLOOR, IngestError, Intake, ingest};

    use super::*;

    fn reading(payload: ExtractedInvoice, confidence: f32) -> Reading {
        Reading {
            extracted_by: "test-model".into(),
            payload,
            reported_confidence: Some(confidence),
        }
    }

    fn pending_document(store: &mut Store) -> DocumentId {
        let dir = tempfile::tempdir().unwrap();
        let Intake::Stored(doc) = ingest(store, dir.path(), upload(b"bytes")).unwrap() else {
            panic!("fresh store");
        };
        doc.id
    }

    #[test]
    fn a_clean_confident_reading_lands_as_extracted() {
        let mut store = store();
        let rules = rules();
        let doc = pending_document(&mut store);

        let recorded = record_reading(
            &mut store,
            doc,
            reading(clean_invoice(), 0.95),
            &rules,
            today(),
            DEFAULT_CONFIDENCE_FLOOR,
        )
        .unwrap();

        assert_eq!(recorded.document_status, DocumentStatus::Extracted);
        assert_eq!(recorded.extraction.version, 1);
        assert!(recorded.extraction.issues.is_empty());
        assert_eq!(store.document(doc).unwrap().status, DocumentStatus::Extracted);
    }

    #[test]
    fn broken_arithmetic_goes_to_review_however_confident() {
        let mut store = store();
        let rules = rules();
        let doc = pending_document(&mut store);

        let mut invoice = clean_invoice();
        invoice.total = Money::nzd(99900);
        invoice.lines.clear();

        let recorded = record_reading(
            &mut store,
            doc,
            reading(invoice, 0.99),
            &rules,
            today(),
            DEFAULT_CONFIDENCE_FLOOR,
        )
        .unwrap();

        assert_eq!(recorded.document_status, DocumentStatus::NeedsReview);
        assert!(recorded.extraction.has_errors());
    }

    #[test]
    fn a_second_reading_supersedes_and_can_clear_review() {
        let mut store = store();
        let rules = rules();
        let doc = pending_document(&mut store);

        record_reading(
            &mut store,
            doc,
            reading(clean_invoice(), 0.3), // clean but unsure → review
            &rules,
            today(),
            DEFAULT_CONFIDENCE_FLOOR,
        )
        .unwrap();
        assert_eq!(store.document(doc).unwrap().status, DocumentStatus::NeedsReview);

        // A human confirms the numbers.
        let recorded = record_reading(
            &mut store,
            doc,
            Reading {
                extracted_by: "human".into(),
                payload: clean_invoice(),
                reported_confidence: None,
            },
            &rules,
            today(),
            DEFAULT_CONFIDENCE_FLOOR,
        )
        .unwrap();

        assert_eq!(recorded.extraction.version, 2);
        assert_eq!(recorded.document_status, DocumentStatus::Extracted);
        assert_eq!(store.extractions_for(doc).unwrap().len(), 2);
    }

    #[test]
    fn ignored_documents_are_not_read() {
        let mut store = store();
        let rules = rules();
        let doc = pending_document(&mut store);
        store.set_document_status(doc, DocumentStatus::Ignored).unwrap();

        let err = record_reading(
            &mut store,
            doc,
            reading(clean_invoice(), 0.9),
            &rules,
            today(),
            DEFAULT_CONFIDENCE_FLOOR,
        )
        .unwrap_err();
        assert!(matches!(err, IngestError::NotReady { .. }));
    }
}
