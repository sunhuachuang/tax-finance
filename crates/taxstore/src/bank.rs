use rusqlite::{OptionalExtension, Row, params};
use taxcore::{BankTransaction, BankTxnId, Currency, DocumentId, EntryId, Money};

use crate::error::not_found;
use crate::{Result, Store, conv_err};

impl Store {
    /// Idempotent import: rows whose dedup hash is already present are
    /// skipped, which is what makes re-exporting an overlapping date range
    /// safe. Returns how many rows were actually new.
    pub fn import_bank_txns(&mut self, txns: &[BankTransaction]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut inserted = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO bank_txns
                   (id, account, date, amount_cents, currency, description,
                    reference, import_batch, dedup_hash, matched_document, entry_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?;
            for txn in txns {
                inserted += stmt.execute(params![
                    txn.id.to_string(),
                    txn.account,
                    txn.date.to_string(),
                    txn.amount.cents,
                    txn.amount.currency.as_str(),
                    txn.description,
                    txn.reference,
                    txn.import_batch,
                    txn.dedup_hash,
                    txn.matched_document.map(|d| d.to_string()),
                    txn.entry.map(|e| e.to_string()),
                ])?;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    pub fn bank_txn(&self, id: BankTxnId) -> Result<BankTransaction> {
        self.conn
            .query_row(
                "SELECT id, account, date, amount_cents, currency, description,
                        reference, import_batch, dedup_hash, matched_document, entry_id
                 FROM bank_txns WHERE id = ?1",
                [id.to_string()],
                row_to_bank_txn,
            )
            .optional()?
            .ok_or_else(|| not_found("bank transaction", id))
    }

    /// The reconciliation worklist: statement rows no ledger entry accounts
    /// for yet. Statements are the completeness spine, so this reaching empty
    /// for a period is what "the books are complete" means.
    pub fn unreconciled_bank_txns(&self) -> Result<Vec<BankTransaction>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account, date, amount_cents, currency, description,
                    reference, import_batch, dedup_hash, matched_document, entry_id
             FROM bank_txns WHERE entry_id IS NULL ORDER BY date, id",
        )?;
        let txns = stmt
            .query_map([], row_to_bank_txn)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(txns)
    }

    /// Documents some bank row already claims. Reconciliation skips these so
    /// one receipt is never proposed against two payments.
    pub fn matched_document_ids(&self) -> Result<Vec<DocumentId>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT matched_document FROM bank_txns
             WHERE matched_document IS NOT NULL",
        )?;
        let ids = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                id.parse::<DocumentId>().map_err(conv_err)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    /// Attach a matched document and/or the entry that accounts for the row.
    /// Passing `None` leaves that link as it is; links are set, not cleared.
    pub fn link_bank_txn(
        &mut self,
        id: BankTxnId,
        document: Option<DocumentId>,
        entry: Option<EntryId>,
    ) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE bank_txns SET
               matched_document = COALESCE(?2, matched_document),
               entry_id = COALESCE(?3, entry_id)
             WHERE id = ?1",
            params![
                id.to_string(),
                document.map(|d| d.to_string()),
                entry.map(|e| e.to_string()),
            ],
        )?;
        if changed == 0 {
            return Err(not_found("bank transaction", id));
        }
        Ok(())
    }
}

fn row_to_bank_txn(row: &Row) -> rusqlite::Result<BankTransaction> {
    let id: String = row.get("id")?;
    let date: String = row.get("date")?;
    let currency: String = row.get("currency")?;
    let matched_document: Option<String> = row.get("matched_document")?;
    let entry: Option<String> = row.get("entry_id")?;
    Ok(BankTransaction {
        id: id.parse().map_err(conv_err)?,
        account: row.get("account")?,
        date: date.parse().map_err(conv_err)?,
        amount: Money::new(
            row.get("amount_cents")?,
            Currency::new(&currency).map_err(conv_err)?,
        ),
        description: row.get("description")?,
        reference: row.get("reference")?,
        import_batch: row.get("import_batch")?,
        dedup_hash: row.get("dedup_hash")?,
        matched_document: matched_document
            .map(|d| d.parse())
            .transpose()
            .map_err(conv_err)?,
        entry: entry.map(|e| e.parse()).transpose().map_err(conv_err)?,
    })
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use taxcore::{DocumentSource, DocumentStatus};

    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn txn(day: u32, cents: i64, batch: &str) -> BankTransaction {
        BankTransaction::new(
            "asb-8842",
            d(2025, 8, day),
            Money::nzd(cents),
            "OFFICEWORKS AUCKLAND",
            batch,
        )
    }

    #[test]
    fn reimports_of_overlapping_ranges_are_idempotent() {
        let mut store = Store::open_in_memory().unwrap();

        let august = vec![txn(14, -11500, "aug"), txn(20, -4999, "aug")];
        assert_eq!(store.import_bank_txns(&august).unwrap(), 2);

        // The next month's export overlaps the previous one.
        let september = vec![txn(20, -4999, "sep"), txn(3, -2500, "sep")];
        assert_eq!(store.import_bank_txns(&september).unwrap(), 1);

        assert_eq!(store.unreconciled_bank_txns().unwrap().len(), 3);
    }

    #[test]
    fn linking_marks_a_row_reconciled() {
        let mut store = Store::open_in_memory().unwrap();
        let row = txn(14, -11500, "aug");
        store.import_bank_txns(std::slice::from_ref(&row)).unwrap();

        let document = taxcore::Document {
            id: DocumentId::new(),
            sha256: "sha-1".into(),
            source: DocumentSource::Upload,
            mime: "image/jpeg".into(),
            byte_len: 1024,
            stored_path: "docs/sh/sha-1.jpg".into(),
            original_filename: None,
            received_at: chrono::Utc::now(),
            status: DocumentStatus::PendingExtraction,
        };
        store.insert_document(&document).unwrap();

        store.link_bank_txn(row.id, Some(document.id), None).unwrap();
        let loaded = store.bank_txn(row.id).unwrap();
        assert_eq!(loaded.matched_document, Some(document.id));
        assert!(loaded.entry.is_none());
        assert_eq!(store.unreconciled_bank_txns().unwrap().len(), 1);

        let missing = store.link_bank_txn(BankTxnId::new(), None, None);
        assert!(matches!(missing.unwrap_err(), crate::StoreError::NotFound { .. }));
    }
}
