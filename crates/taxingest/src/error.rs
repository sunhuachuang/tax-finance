use taxcore::DocumentId;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, IngestError>;

#[derive(Debug, Error)]
pub enum IngestError {
    #[error(transparent)]
    Store(#[from] taxstore::StoreError),

    #[error(transparent)]
    Tax(#[from] taxcore::TaxError),

    #[error("cannot read or write the document file: {0}")]
    Io(#[from] std::io::Error),

    #[error("document {document} is not ready: {reason}")]
    NotReady {
        document: DocumentId,
        reason: String,
    },

    #[error("cannot draft a {0} entry; the ledger is NZD and there is no rate source yet")]
    UnsupportedCurrency(String),

    #[error("account {0} is retired")]
    RetiredAccount(String),
}

pub(crate) fn not_ready(document: DocumentId, reason: impl Into<String>) -> IngestError {
    IngestError::NotReady {
        document,
        reason: reason.into(),
    }
}
