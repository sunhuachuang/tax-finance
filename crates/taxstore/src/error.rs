use thiserror::Error;

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error("stored value does not round-trip: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Domain(#[from] taxcore::TaxError),

    #[error("{what} {id} not found")]
    NotFound { what: &'static str, id: String },

    #[error("invalid transition: {0}")]
    InvalidTransition(String),

    #[error("extraction version must be {expected}, got {got}")]
    ExtractionVersion { expected: u32, got: u32 },

    #[error("journal entry does not balance")]
    UnbalancedEntry,

    #[error("an entry with no provenance sources is a defect, not a special case")]
    EmptyProvenance,

    #[error("provenance names entry {provenance} but the entry is {entry}")]
    ProvenanceMismatch { provenance: String, entry: String },

    #[error("database schema is version {0}, newer than this build understands")]
    SchemaTooNew(i64),
}

pub(crate) fn not_found(what: &'static str, id: impl ToString) -> StoreError {
    StoreError::NotFound {
        what,
        id: id.to_string(),
    }
}
