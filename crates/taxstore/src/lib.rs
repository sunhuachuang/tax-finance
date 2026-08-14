//! SQLite persistence for the tax engine.
//!
//! Append-only by construction and by trigger: entries and their postings are
//! never updated or deleted — corrections are reversing entries. Extractions
//! are superseded, never overwritten. Documents change only their status, and
//! bank rows only their match links. The triggers exist so that even a bug in
//! this crate cannot silently rewrite history.

mod accounts;
mod bank;
mod documents;
mod error;
mod ledger;
mod schema;

pub use error::{Result, StoreError};

use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
        // journal_mode returns the resulting mode as a row, so it cannot go
        // through execute_batch.
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        schema::migrate(&mut conn)?;
        Ok(Store { conn })
    }
}

/// Unit enums serialize to a bare snake_case token, stored without the JSON
/// quotes so the column reads naturally in a query.
pub(crate) fn enum_to_text<T: Serialize>(value: &T) -> Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(s) => Ok(s),
        other => Ok(other.to_string()),
    }
}

pub(crate) fn enum_from_text<T: DeserializeOwned>(text: &str) -> Result<T> {
    Ok(serde_json::from_value(serde_json::Value::String(
        text.to_string(),
    ))?)
}

/// Lift a conversion failure into rusqlite's error type so row-mapping
/// closures can use `?`.
pub(crate) fn conv_err<E>(e: E) -> rusqlite::Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
}
