use thiserror::Error;

pub type Result<T> = std::result::Result<T, ReturnError>;

#[derive(Debug, Error)]
pub enum ReturnError {
    #[error(transparent)]
    Store(#[from] taxstore::StoreError),

    #[error(transparent)]
    Tax(#[from] taxcore::TaxError),

    #[error(transparent)]
    Rules(#[from] taxrules::RulesError),

    #[error(
        "entry {entry} posts to account {code}, which is not in the chart — \
         it cannot be classified for a return"
    )]
    UnknownAccount { code: String, entry: String },

    #[error("the rule file is for tax year {rules}, but the report covers {wanted}")]
    WrongRulesYear { rules: String, wanted: String },
}
