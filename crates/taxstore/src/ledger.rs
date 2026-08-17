use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{OptionalExtension, Row, Transaction, params};
use taxcore::{
    Currency, Entry, EntryId, EntrySource, EntryStatus, Money, Posting, Provenance, TaxError,
};

use crate::error::not_found;
use crate::{Result, Store, StoreError, conv_err, enum_from_text, enum_to_text};

impl Store {
    /// An entry and its provenance land together or not at all. There is no
    /// way to store an entry without saying where it came from.
    pub fn insert_entry(&mut self, entry: &Entry, provenance: &Provenance) -> Result<()> {
        check_entry(entry, provenance)?;
        let tx = self.conn.transaction()?;
        write_entry(&tx, entry)?;
        write_provenance(&tx, provenance)?;
        tx.commit()?;
        Ok(())
    }

    pub fn entry(&self, id: EntryId) -> Result<Entry> {
        let mut entry = self
            .conn
            .query_row(
                "SELECT id, date, narration, status, source, created_at, reverses
                 FROM entries WHERE id = ?1",
                [id.to_string()],
                row_to_entry_header,
            )
            .optional()?
            .ok_or_else(|| not_found("entry", id))?;
        self.load_postings(&mut entry)?;
        Ok(entry)
    }

    pub fn provenance(&self, entry: EntryId) -> Result<Provenance> {
        self.conn
            .query_row(
                "SELECT sources, note FROM provenance WHERE entry_id = ?1",
                [entry.to_string()],
                |row| {
                    let sources: String = row.get("sources")?;
                    Ok(Provenance {
                        entry,
                        sources: serde_json::from_str(&sources).map_err(conv_err)?,
                        note: row.get("note")?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| not_found("provenance for entry", entry))
    }

    /// The review queue is `entries_with_status(EntryStatus::Draft)`.
    pub fn entries_with_status(&self, status: EntryStatus) -> Result<Vec<Entry>> {
        self.collect_entries(
            "SELECT id, date, narration, status, source, created_at, reverses
             FROM entries WHERE status = ?1 ORDER BY date, id",
            [enum_to_text(&status)?],
        )
    }

    /// What return generation reads: the posted ledger over a period, both
    /// bounds inclusive. Reversed entries are excluded — their effect is
    /// represented by the reversal pair having been posted.
    /// Everything that is part of a return over this range.
    ///
    /// `reversed` belongs here alongside `posted`. A reversed entry really was
    /// posted; what cancels it is its reversing entry, which is itself a
    /// `posted` row on the date the correction was made. Dropping the original
    /// while keeping the reversal would subtract a figure that was never added
    /// — and worse, would silently change a return that has already been filed
    /// the moment someone corrects a later mistake. The pair nets to zero on
    /// its own, which is exactly what `taxreturn::scan` assumes.
    ///
    /// `draft` and `voided` stay out: neither was ever part of a return.
    pub fn posted_entries_between(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<Entry>> {
        self.collect_entries(
            "SELECT id, date, narration, status, source, created_at, reverses
             FROM entries WHERE status IN ('posted', 'reversed') AND date >= ?1 AND date <= ?2
             ORDER BY date, id",
            [from.to_string(), to.to_string()],
        )
    }

    pub fn post_entry(&mut self, id: EntryId) -> Result<()> {
        let entry = self.entry(id)?;
        if entry.status != EntryStatus::Draft {
            return Err(StoreError::InvalidTransition(format!(
                "entry {id} is {}, only a draft can be posted",
                enum_to_text(&entry.status)?
            )));
        }
        self.conn.execute(
            "UPDATE entries SET status = 'posted' WHERE id = ?1",
            [id.to_string()],
        )?;
        Ok(())
    }

    /// The review queue's reject action. The row stays — what the agent
    /// proposed is part of history — it just never becomes part of a return.
    pub fn void_entry(&mut self, id: EntryId) -> Result<()> {
        let entry = self.entry(id)?;
        if entry.status != EntryStatus::Draft {
            return Err(StoreError::InvalidTransition(format!(
                "entry {id} is {}, only a draft can be voided",
                enum_to_text(&entry.status)?
            )));
        }
        self.conn.execute(
            "UPDATE entries SET status = 'voided' WHERE id = ?1",
            [id.to_string()],
        )?;
        Ok(())
    }

    /// The only correction primitive. Inserts the mirror-image entry (already
    /// posted, carrying the original's sources as provenance) and marks the
    /// original reversed, atomically.
    pub fn reverse_entry(
        &mut self,
        id: EntryId,
        date: NaiveDate,
        source: EntrySource,
        note: Option<String>,
    ) -> Result<Entry> {
        let original = self.entry(id)?;
        if original.status != EntryStatus::Posted {
            return Err(StoreError::InvalidTransition(format!(
                "entry {id} is {}, only a posted entry can be reversed",
                enum_to_text(&original.status)?
            )));
        }
        let sources = self.provenance(id)?.sources;
        let reversal = original.reversal(date, source);
        let mut provenance = Provenance::new(reversal.id, sources);
        provenance.note = note.or_else(|| Some(format!("reversal of entry {id}")));

        let tx = self.conn.transaction()?;
        write_entry(&tx, &reversal)?;
        write_provenance(&tx, &provenance)?;
        tx.execute(
            "UPDATE entries SET status = 'reversed' WHERE id = ?1",
            [id.to_string()],
        )?;
        tx.commit()?;
        Ok(reversal)
    }

    fn collect_entries<P: rusqlite::Params>(&self, sql: &str, params: P) -> Result<Vec<Entry>> {
        let mut stmt = self.conn.prepare(sql)?;
        let mut entries = stmt
            .query_map(params, row_to_entry_header)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for entry in &mut entries {
            self.load_postings(entry)?;
        }
        Ok(entries)
    }

    fn load_postings(&self, entry: &mut Entry) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account, amount_cents, currency, gst_treatment,
                    gst_cents, foreign_amount, memo
             FROM postings WHERE entry_id = ?1 ORDER BY seq",
        )?;
        entry.postings = stmt
            .query_map([entry.id.to_string()], row_to_posting)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(())
    }
}

fn check_entry(entry: &Entry, provenance: &Provenance) -> Result<()> {
    if entry.postings.len() < 2 {
        return Err(TaxError::TooFewPostings(entry.postings.len()).into());
    }
    if !entry.is_balanced() {
        return Err(StoreError::UnbalancedEntry);
    }
    for posting in &entry.postings {
        if let Some(gst) = posting.gst_amount
            && gst.currency != posting.amount.currency
        {
            // The gst_cents column shares the posting's currency; a mixed
            // posting would not survive the round trip.
            return Err(TaxError::CurrencyMismatch {
                left: posting.amount.currency.to_string(),
                right: gst.currency.to_string(),
            }
            .into());
        }
    }
    if provenance.entry != entry.id {
        return Err(StoreError::ProvenanceMismatch {
            provenance: provenance.entry.to_string(),
            entry: entry.id.to_string(),
        });
    }
    if provenance.sources.is_empty() {
        return Err(StoreError::EmptyProvenance);
    }
    Ok(())
}

fn write_entry(tx: &Transaction, entry: &Entry) -> Result<()> {
    tx.execute(
        "INSERT INTO entries (id, date, narration, status, source, created_at, reverses)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            entry.id.to_string(),
            entry.date.to_string(),
            entry.narration,
            enum_to_text(&entry.status)?,
            serde_json::to_string(&entry.source)?,
            entry.created_at.to_rfc3339(),
            entry.reverses.map(|r| r.to_string()),
        ],
    )?;
    for (seq, posting) in entry.postings.iter().enumerate() {
        tx.execute(
            "INSERT INTO postings
               (id, entry_id, seq, account, amount_cents, currency,
                gst_treatment, gst_cents, foreign_amount, memo)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                posting.id.to_string(),
                entry.id.to_string(),
                seq as i64,
                posting.account.as_str(),
                posting.amount.cents,
                posting.amount.currency.as_str(),
                enum_to_text(&posting.gst_treatment)?,
                posting.gst_amount.map(|g| g.cents),
                posting
                    .foreign
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                posting.memo,
            ],
        )?;
    }
    Ok(())
}

fn write_provenance(tx: &Transaction, provenance: &Provenance) -> Result<()> {
    tx.execute(
        "INSERT INTO provenance (entry_id, sources, note) VALUES (?1, ?2, ?3)",
        params![
            provenance.entry.to_string(),
            serde_json::to_string(&provenance.sources)?,
            provenance.note,
        ],
    )?;
    Ok(())
}

fn row_to_entry_header(row: &Row) -> rusqlite::Result<Entry> {
    let id: String = row.get("id")?;
    let date: String = row.get("date")?;
    let status: String = row.get("status")?;
    let source: String = row.get("source")?;
    let created_at: String = row.get("created_at")?;
    let reverses: Option<String> = row.get("reverses")?;
    Ok(Entry {
        id: id.parse().map_err(conv_err)?,
        date: date.parse().map_err(conv_err)?,
        narration: row.get("narration")?,
        postings: Vec::new(),
        status: enum_from_text(&status).map_err(conv_err)?,
        source: serde_json::from_str(&source).map_err(conv_err)?,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map_err(conv_err)?
            .with_timezone(&Utc),
        reverses: reverses.map(|r| r.parse()).transpose().map_err(conv_err)?,
    })
}

fn row_to_posting(row: &Row) -> rusqlite::Result<Posting> {
    let id: String = row.get("id")?;
    let account: String = row.get("account")?;
    let currency: String = row.get("currency")?;
    let gst_treatment: String = row.get("gst_treatment")?;
    let foreign: Option<String> = row.get("foreign_amount")?;
    let currency = Currency::new(&currency).map_err(conv_err)?;
    Ok(Posting {
        id: id.parse().map_err(conv_err)?,
        account: account.parse().map_err(conv_err)?,
        amount: Money::new(row.get("amount_cents")?, currency),
        gst_treatment: enum_from_text(&gst_treatment).map_err(conv_err)?,
        gst_amount: row
            .get::<_, Option<i64>>("gst_cents")?
            .map(|cents| Money::new(cents, currency)),
        foreign: foreign
            .map(|f| serde_json::from_str(&f))
            .transpose()
            .map_err(conv_err)?,
        memo: row.get("memo")?,
    })
}

#[cfg(test)]
mod tests {
    use taxcore::{AccountCode, EntryBuilder, GstTreatment, SourceRef};

    use super::*;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn code(s: &str) -> AccountCode {
        AccountCode::new(s).unwrap()
    }

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 8, 14).unwrap()
    }

    fn agent() -> EntrySource {
        EntrySource::Agent {
            model: "test-model".into(),
        }
    }

    fn office_entry() -> Entry {
        EntryBuilder::new(date(), "Officeworks paper", agent())
            .posting(
                Posting::new(code("6100-office"), Money::nzd(11500), GstTreatment::Standard)
                    .with_gst_amount(Money::nzd(1500))
                    .with_memo("A4 paper"),
            )
            .credit(code("1010-bank"), Money::nzd(11500), GstTreatment::NotSubject)
            .build()
            .unwrap()
    }

    fn manual_provenance(entry: &Entry) -> Provenance {
        Provenance::new(entry.id, vec![SourceRef::Manual])
    }

    #[test]
    fn an_entry_round_trips_with_postings_and_provenance() {
        let mut store = store();
        let entry = office_entry();
        store.insert_entry(&entry, &manual_provenance(&entry)).unwrap();

        let loaded = store.entry(entry.id).unwrap();
        assert_eq!(loaded, entry);

        let provenance = store.provenance(entry.id).unwrap();
        assert_eq!(provenance.sources, vec![SourceRef::Manual]);
    }

    #[test]
    fn provenance_is_mandatory_and_must_match() {
        let mut store = store();
        let entry = office_entry();

        let empty = Provenance::new(entry.id, vec![]);
        assert!(matches!(
            store.insert_entry(&entry, &empty).unwrap_err(),
            StoreError::EmptyProvenance
        ));

        let other = Provenance::new(EntryId::new(), vec![SourceRef::Manual]);
        assert!(matches!(
            store.insert_entry(&entry, &other).unwrap_err(),
            StoreError::ProvenanceMismatch { .. }
        ));
    }

    #[test]
    fn an_unbalanced_entry_never_reaches_disk() {
        let mut store = store();
        let mut entry = office_entry();
        entry.postings[0].amount = Money::nzd(11400);
        assert!(matches!(
            store
                .insert_entry(&entry, &manual_provenance(&entry))
                .unwrap_err(),
            StoreError::UnbalancedEntry
        ));
    }

    #[test]
    fn drafts_post_and_only_drafts() {
        let mut store = store();
        let entry = office_entry();
        store.insert_entry(&entry, &manual_provenance(&entry)).unwrap();

        store.post_entry(entry.id).unwrap();
        assert_eq!(store.entry(entry.id).unwrap().status, EntryStatus::Posted);

        assert!(matches!(
            store.post_entry(entry.id).unwrap_err(),
            StoreError::InvalidTransition(_)
        ));
    }

    #[test]
    fn rejected_drafts_are_voided_not_deleted() {
        let mut store = store();
        let entry = office_entry();
        store.insert_entry(&entry, &manual_provenance(&entry)).unwrap();

        store.void_entry(entry.id).unwrap();
        let voided = store.entry(entry.id).unwrap();
        assert_eq!(voided.status, EntryStatus::Voided);

        // A voided draft can be neither posted nor voided again.
        assert!(matches!(
            store.post_entry(entry.id).unwrap_err(),
            StoreError::InvalidTransition(_)
        ));
        assert!(matches!(
            store.void_entry(entry.id).unwrap_err(),
            StoreError::InvalidTransition(_)
        ));
    }

    #[test]
    fn reversal_is_atomic_and_carries_provenance() {
        let mut store = store();
        let entry = office_entry();
        store.insert_entry(&entry, &manual_provenance(&entry)).unwrap();
        store.post_entry(entry.id).unwrap();

        let reversal = store
            .reverse_entry(entry.id, date(), EntrySource::Human, None)
            .unwrap();

        assert_eq!(store.entry(entry.id).unwrap().status, EntryStatus::Reversed);
        let stored = store.entry(reversal.id).unwrap();
        assert_eq!(stored.reverses, Some(entry.id));
        assert_eq!(stored.status, EntryStatus::Posted);
        assert!(stored.is_balanced());

        let provenance = store.provenance(reversal.id).unwrap();
        assert_eq!(provenance.sources, vec![SourceRef::Manual]);
        assert!(provenance.note.unwrap().contains("reversal"));

        // A draft cannot be reversed, and a reversed entry cannot be reversed again.
        assert!(matches!(
            store
                .reverse_entry(entry.id, date(), EntrySource::Human, None)
                .unwrap_err(),
            StoreError::InvalidTransition(_)
        ));
    }

    #[test]
    fn the_period_query_sees_posted_entries_only() {
        let mut store = store();
        let posted = office_entry();
        store.insert_entry(&posted, &manual_provenance(&posted)).unwrap();
        store.post_entry(posted.id).unwrap();

        let draft = office_entry();
        store.insert_entry(&draft, &manual_provenance(&draft)).unwrap();

        let from = NaiveDate::from_ymd_opt(2025, 8, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2025, 8, 31).unwrap();
        let entries = store.posted_entries_between(from, to).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, posted.id);

        assert_eq!(store.entries_with_status(EntryStatus::Draft).unwrap().len(), 1);
    }

    #[test]
    fn the_database_itself_refuses_history_edits() {
        let mut store = store();
        let entry = office_entry();
        store.insert_entry(&entry, &manual_provenance(&entry)).unwrap();

        let narration_edit = store.conn.execute(
            "UPDATE entries SET narration = 'rewritten' WHERE id = ?1",
            [entry.id.to_string()],
        );
        assert!(narration_edit.is_err());

        let posting_edit = store.conn.execute(
            "UPDATE postings SET amount_cents = 1 WHERE entry_id = ?1",
            [entry.id.to_string()],
        );
        assert!(posting_edit.is_err());

        let delete = store
            .conn
            .execute("DELETE FROM postings WHERE entry_id = ?1", [entry.id.to_string()]);
        assert!(delete.is_err());
    }
}
