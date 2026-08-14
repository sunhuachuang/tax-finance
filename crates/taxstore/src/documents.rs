use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};
use taxcore::{Document, DocumentId, DocumentStatus, Extraction};

use crate::error::not_found;
use crate::{Result, Store, StoreError, conv_err, enum_from_text, enum_to_text};

/// The document lifecycle. Anything outside these edges — most importantly any
/// path that resurrects an `Ignored` document straight into `Extracted` — has
/// to go back through the queue.
fn transition_allowed(from: DocumentStatus, to: DocumentStatus) -> bool {
    use DocumentStatus::*;
    matches!(
        (from, to),
        (PendingExtraction, Extracted | NeedsReview | Ignored)
            | (NeedsReview, Extracted | Ignored)
            | (Extracted, NeedsReview | Ignored)
            | (Ignored, PendingExtraction)
    )
}

impl Store {
    pub fn insert_document(&mut self, doc: &Document) -> Result<()> {
        self.conn.execute(
            "INSERT INTO documents
               (id, sha256, source, mime, byte_len, stored_path, original_filename, received_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                doc.id.to_string(),
                doc.sha256,
                serde_json::to_string(&doc.source)?,
                doc.mime,
                doc.byte_len as i64,
                doc.stored_path,
                doc.original_filename,
                doc.received_at.to_rfc3339(),
                enum_to_text(&doc.status)?,
            ],
        )?;
        Ok(())
    }

    pub fn document(&self, id: DocumentId) -> Result<Document> {
        self.conn
            .query_row(
                "SELECT id, sha256, source, mime, byte_len, stored_path,
                        original_filename, received_at, status
                 FROM documents WHERE id = ?1",
                [id.to_string()],
                row_to_document,
            )
            .optional()?
            .ok_or_else(|| not_found("document", id))
    }

    /// The dedup gate: a document whose bytes are already stored comes back
    /// here instead of being stored twice.
    pub fn document_by_sha256(&self, sha256: &str) -> Result<Option<Document>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, sha256, source, mime, byte_len, stored_path,
                        original_filename, received_at, status
                 FROM documents WHERE sha256 = ?1",
                [sha256],
                row_to_document,
            )
            .optional()?)
    }

    pub fn documents_with_status(&self, status: DocumentStatus) -> Result<Vec<Document>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, sha256, source, mime, byte_len, stored_path,
                    original_filename, received_at, status
             FROM documents WHERE status = ?1 ORDER BY id",
        )?;
        let docs = stmt
            .query_map([enum_to_text(&status)?], row_to_document)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(docs)
    }

    pub fn set_document_status(&mut self, id: DocumentId, to: DocumentStatus) -> Result<()> {
        let from = self.document(id)?.status;
        if from == to {
            return Ok(());
        }
        if !transition_allowed(from, to) {
            return Err(StoreError::InvalidTransition(format!(
                "document {id}: {} -> {}",
                enum_to_text(&from)?,
                enum_to_text(&to)?
            )));
        }
        self.conn.execute(
            "UPDATE documents SET status = ?1 WHERE id = ?2",
            params![enum_to_text(&to)?, id.to_string()],
        )?;
        Ok(())
    }

    /// Versions are dense and strictly increasing per document; inserting a new
    /// one supersedes whatever was current. The version is caller-supplied
    /// rather than assigned here so an [`Extraction`] is identical in memory
    /// and at rest.
    pub fn insert_extraction(&mut self, extraction: &Extraction) -> Result<()> {
        let doc_id = extraction.document_id.to_string();
        let tx = self.conn.transaction()?;

        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM documents WHERE id = ?1)",
            [&doc_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(not_found("document", extraction.document_id));
        }

        let latest: u32 = tx.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM extractions WHERE document_id = ?1",
            [&doc_id],
            |row| row.get(0),
        )?;
        let expected = latest + 1;
        if extraction.version != expected {
            return Err(StoreError::ExtractionVersion {
                expected,
                got: extraction.version,
            });
        }

        tx.execute(
            "UPDATE extractions SET superseded = 1 WHERE document_id = ?1 AND superseded = 0",
            [&doc_id],
        )?;
        tx.execute(
            "INSERT INTO extractions
               (id, document_id, version, extracted_by, extracted_at,
                payload, reported_confidence, issues, superseded)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0)",
            params![
                extraction.id.to_string(),
                doc_id,
                extraction.version,
                extraction.extracted_by,
                extraction.extracted_at.to_rfc3339(),
                serde_json::to_string(&extraction.payload)?,
                extraction.reported_confidence,
                serde_json::to_string(&extraction.issues)?,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn latest_extraction(&self, document: DocumentId) -> Result<Option<Extraction>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, document_id, version, extracted_by, extracted_at,
                        payload, reported_confidence, issues, superseded
                 FROM extractions WHERE document_id = ?1 AND superseded = 0",
                [document.to_string()],
                row_to_extraction,
            )
            .optional()?)
    }

    pub fn extractions_for(&self, document: DocumentId) -> Result<Vec<Extraction>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, document_id, version, extracted_by, extracted_at,
                    payload, reported_confidence, issues, superseded
             FROM extractions WHERE document_id = ?1 ORDER BY version",
        )?;
        let extractions = stmt
            .query_map([document.to_string()], row_to_extraction)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(extractions)
    }
}

fn row_to_document(row: &Row) -> rusqlite::Result<Document> {
    let id: String = row.get("id")?;
    let source: String = row.get("source")?;
    let received_at: String = row.get("received_at")?;
    let status: String = row.get("status")?;
    Ok(Document {
        id: id.parse().map_err(conv_err)?,
        sha256: row.get("sha256")?,
        source: serde_json::from_str(&source).map_err(conv_err)?,
        mime: row.get("mime")?,
        byte_len: row.get::<_, i64>("byte_len")? as u64,
        stored_path: row.get("stored_path")?,
        original_filename: row.get("original_filename")?,
        received_at: DateTime::parse_from_rfc3339(&received_at)
            .map_err(conv_err)?
            .with_timezone(&Utc),
        status: enum_from_text(&status).map_err(conv_err)?,
    })
}

fn row_to_extraction(row: &Row) -> rusqlite::Result<Extraction> {
    let id: String = row.get("id")?;
    let document_id: String = row.get("document_id")?;
    let extracted_at: String = row.get("extracted_at")?;
    let payload: String = row.get("payload")?;
    let issues: String = row.get("issues")?;
    Ok(Extraction {
        id: id.parse().map_err(conv_err)?,
        document_id: document_id.parse().map_err(conv_err)?,
        version: row.get("version")?,
        extracted_by: row.get("extracted_by")?,
        extracted_at: DateTime::parse_from_rfc3339(&extracted_at)
            .map_err(conv_err)?
            .with_timezone(&Utc),
        payload: serde_json::from_str(&payload).map_err(conv_err)?,
        reported_confidence: row.get("reported_confidence")?,
        issues: serde_json::from_str(&issues).map_err(conv_err)?,
        superseded: row.get("superseded")?,
    })
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use taxcore::{Currency, DocumentSource, ExtractedInvoice, GstRate, LineItem, Money};

    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn doc(sha: &str) -> Document {
        Document {
            id: DocumentId::new(),
            sha256: sha.to_string(),
            source: DocumentSource::Email {
                from: "billing@officeworks.co.nz".into(),
                subject: "Invoice INV-8842".into(),
                message_id: "<msg-1@officeworks>".into(),
            },
            mime: "application/pdf".into(),
            byte_len: 48_213,
            stored_path: "docs/ab/abc.pdf".into(),
            original_filename: Some("INV-8842.pdf".into()),
            received_at: Utc::now(),
            status: DocumentStatus::PendingExtraction,
        }
    }

    fn invoice() -> ExtractedInvoice {
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

    fn extraction(document: DocumentId, version: u32) -> Extraction {
        Extraction::new(
            document,
            version,
            "test-model",
            invoice(),
            GstRate::new(15, 100),
            NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
        )
    }

    #[test]
    fn a_document_round_trips() {
        let mut store = store();
        let doc = doc("sha-1");
        store.insert_document(&doc).unwrap();

        let loaded = store.document(doc.id).unwrap();
        assert_eq!(loaded.sha256, doc.sha256);
        assert_eq!(loaded.source, doc.source);
        assert_eq!(loaded.received_at, doc.received_at);
        assert_eq!(loaded.status, DocumentStatus::PendingExtraction);
    }

    #[test]
    fn duplicate_bytes_are_rejected_and_findable() {
        let mut store = store();
        let original = doc("sha-1");
        store.insert_document(&original).unwrap();

        assert!(store.insert_document(&doc("sha-1")).is_err());
        let found = store.document_by_sha256("sha-1").unwrap().unwrap();
        assert_eq!(found.id, original.id);
        assert!(store.document_by_sha256("sha-2").unwrap().is_none());
    }

    #[test]
    fn the_status_state_machine_is_enforced() {
        let mut store = store();
        let doc = doc("sha-1");
        store.insert_document(&doc).unwrap();

        store
            .set_document_status(doc.id, DocumentStatus::NeedsReview)
            .unwrap();
        store
            .set_document_status(doc.id, DocumentStatus::Ignored)
            .unwrap();

        // An ignored document cannot jump straight to extracted.
        let err = store
            .set_document_status(doc.id, DocumentStatus::Extracted)
            .unwrap_err();
        assert!(matches!(err, StoreError::InvalidTransition(_)));

        // It has to go back through the queue.
        store
            .set_document_status(doc.id, DocumentStatus::PendingExtraction)
            .unwrap();
        store
            .set_document_status(doc.id, DocumentStatus::Extracted)
            .unwrap();
    }

    #[test]
    fn extraction_versions_are_dense_and_supersede() {
        let mut store = store();
        let doc = doc("sha-1");
        store.insert_document(&doc).unwrap();

        let first = extraction(doc.id, 1);
        store.insert_extraction(&first).unwrap();

        let err = store.insert_extraction(&extraction(doc.id, 3)).unwrap_err();
        assert!(matches!(
            err,
            StoreError::ExtractionVersion {
                expected: 2,
                got: 3
            }
        ));

        let second = extraction(doc.id, 2);
        store.insert_extraction(&second).unwrap();

        let latest = store.latest_extraction(doc.id).unwrap().unwrap();
        assert_eq!(latest.id, second.id);
        assert_eq!(latest.payload, second.payload);

        let all = store.extractions_for(doc.id).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[0].superseded);
        assert!(!all[1].superseded);
    }

    #[test]
    fn an_extraction_for_a_missing_document_is_refused() {
        let mut store = store();
        let err = store
            .insert_extraction(&extraction(DocumentId::new(), 1))
            .unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }
}
