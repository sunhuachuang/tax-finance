use rusqlite::Connection;

use crate::{Result, StoreError};

const CURRENT_VERSION: i64 = 1;

pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > CURRENT_VERSION {
        return Err(StoreError::SchemaTooNew(version));
    }
    if version < 1 {
        let tx = conn.transaction()?;
        tx.execute_batch(V1)?;
        tx.pragma_update(None, "user_version", 1)?;
        tx.commit()?;
    }
    Ok(())
}

// Dates are ISO-8601 TEXT. Ids are uuid TEXT (v7, so lexical order is creation
// order). Money is minor-unit INTEGER plus a currency column; a posting's GST
// content shares the posting's currency, checked on insert. Structured enums
// (document source, entry source, provenance sources, validation issues,
// foreign amounts) are JSON TEXT — they are read back whole, never queried into.
const V1: &str = "
CREATE TABLE documents (
  id                TEXT PRIMARY KEY,
  sha256            TEXT NOT NULL UNIQUE,
  source            TEXT NOT NULL,
  mime              TEXT NOT NULL,
  byte_len          INTEGER NOT NULL,
  stored_path       TEXT NOT NULL,
  original_filename TEXT,
  received_at       TEXT NOT NULL,
  status            TEXT NOT NULL
) STRICT;

CREATE TABLE extractions (
  id                  TEXT PRIMARY KEY,
  document_id         TEXT NOT NULL REFERENCES documents(id),
  version             INTEGER NOT NULL,
  extracted_by        TEXT NOT NULL,
  extracted_at        TEXT NOT NULL,
  payload             TEXT NOT NULL,
  reported_confidence REAL,
  issues              TEXT NOT NULL,
  superseded          INTEGER NOT NULL DEFAULT 0,
  UNIQUE (document_id, version)
) STRICT;

CREATE TABLE entries (
  id         TEXT PRIMARY KEY,
  date       TEXT NOT NULL,
  narration  TEXT NOT NULL,
  status     TEXT NOT NULL,
  source     TEXT NOT NULL,
  created_at TEXT NOT NULL,
  reverses   TEXT REFERENCES entries(id)
) STRICT;

CREATE INDEX entries_by_date ON entries(date);

CREATE TABLE postings (
  id             TEXT PRIMARY KEY,
  entry_id       TEXT NOT NULL REFERENCES entries(id),
  seq            INTEGER NOT NULL,
  account        TEXT NOT NULL,
  amount_cents   INTEGER NOT NULL,
  currency       TEXT NOT NULL,
  gst_treatment  TEXT NOT NULL,
  gst_cents      INTEGER,
  foreign_amount TEXT,
  memo           TEXT,
  UNIQUE (entry_id, seq)
) STRICT;

CREATE INDEX postings_by_account ON postings(account);

CREATE TABLE provenance (
  entry_id TEXT PRIMARY KEY REFERENCES entries(id),
  sources  TEXT NOT NULL,
  note     TEXT
) STRICT;

CREATE TABLE accounts (
  code                  TEXT PRIMARY KEY,
  name                  TEXT NOT NULL,
  kind                  TEXT NOT NULL,
  parent                TEXT REFERENCES accounts(code),
  default_gst_treatment TEXT,
  active                INTEGER NOT NULL DEFAULT 1
) STRICT;

CREATE TABLE bank_txns (
  id               TEXT PRIMARY KEY,
  account          TEXT NOT NULL,
  date             TEXT NOT NULL,
  amount_cents     INTEGER NOT NULL,
  currency         TEXT NOT NULL,
  description      TEXT NOT NULL,
  reference        TEXT,
  import_batch     TEXT NOT NULL,
  dedup_hash       TEXT NOT NULL UNIQUE,
  matched_document TEXT REFERENCES documents(id),
  entry_id         TEXT REFERENCES entries(id)
) STRICT;

CREATE INDEX bank_txns_by_account_date ON bank_txns(account, date);

CREATE TRIGGER entries_only_change_status BEFORE UPDATE ON entries
WHEN OLD.id != NEW.id OR OLD.date != NEW.date OR OLD.narration != NEW.narration
  OR OLD.source != NEW.source OR OLD.created_at != NEW.created_at
  OR COALESCE(OLD.reverses, '') != COALESCE(NEW.reverses, '')
BEGIN SELECT RAISE(ABORT, 'entries are append-only; only status may change'); END;

CREATE TRIGGER entries_never_deleted BEFORE DELETE ON entries
BEGIN SELECT RAISE(ABORT, 'entries are never deleted; post a reversal'); END;

CREATE TRIGGER postings_never_updated BEFORE UPDATE ON postings
BEGIN SELECT RAISE(ABORT, 'postings are immutable'); END;

CREATE TRIGGER postings_never_deleted BEFORE DELETE ON postings
BEGIN SELECT RAISE(ABORT, 'postings are never deleted'); END;

CREATE TRIGGER extractions_only_supersede BEFORE UPDATE ON extractions
WHEN OLD.id != NEW.id OR OLD.document_id != NEW.document_id
  OR OLD.version != NEW.version OR OLD.extracted_by != NEW.extracted_by
  OR OLD.extracted_at != NEW.extracted_at OR OLD.payload != NEW.payload
  OR COALESCE(OLD.reported_confidence, -1) != COALESCE(NEW.reported_confidence, -1)
  OR OLD.issues != NEW.issues
BEGIN SELECT RAISE(ABORT, 'extractions are immutable; only superseded may change'); END;

CREATE TRIGGER extractions_never_deleted BEFORE DELETE ON extractions
BEGIN SELECT RAISE(ABORT, 'extractions are never deleted; add a new version'); END;

CREATE TRIGGER documents_only_change_status BEFORE UPDATE ON documents
WHEN OLD.id != NEW.id OR OLD.sha256 != NEW.sha256 OR OLD.source != NEW.source
  OR OLD.mime != NEW.mime OR OLD.byte_len != NEW.byte_len
  OR OLD.stored_path != NEW.stored_path
  OR COALESCE(OLD.original_filename, '') != COALESCE(NEW.original_filename, '')
  OR OLD.received_at != NEW.received_at
BEGIN SELECT RAISE(ABORT, 'documents are immutable; only status may change'); END;

CREATE TRIGGER documents_never_deleted BEFORE DELETE ON documents
BEGIN SELECT RAISE(ABORT, 'documents are never deleted; mark them ignored'); END;

CREATE TRIGGER bank_txns_only_change_links BEFORE UPDATE ON bank_txns
WHEN OLD.id != NEW.id OR OLD.account != NEW.account OR OLD.date != NEW.date
  OR OLD.amount_cents != NEW.amount_cents OR OLD.currency != NEW.currency
  OR OLD.description != NEW.description
  OR COALESCE(OLD.reference, '') != COALESCE(NEW.reference, '')
  OR OLD.import_batch != NEW.import_batch OR OLD.dedup_hash != NEW.dedup_hash
BEGIN SELECT RAISE(ABORT, 'bank rows are immutable; only match links may change'); END;

CREATE TRIGGER bank_txns_never_deleted BEFORE DELETE ON bank_txns
BEGIN SELECT RAISE(ABORT, 'bank rows are never deleted'); END;
";
