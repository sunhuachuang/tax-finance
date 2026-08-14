use rusqlite::{OptionalExtension, Row, params};
use taxcore::{Account, AccountCode};

use crate::error::not_found;
use crate::{Result, Store, conv_err, enum_from_text, enum_to_text};

impl Store {
    /// The chart of accounts is reference data, not history: renaming an
    /// account or retiring it is an edit, so this is an upsert rather than an
    /// append. What must not change is which postings point at which code, and
    /// postings are immutable.
    pub fn upsert_account(&mut self, account: &Account) -> Result<()> {
        self.conn.execute(
            "INSERT INTO accounts (code, name, kind, parent, default_gst_treatment, active)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(code) DO UPDATE SET
               name = excluded.name,
               kind = excluded.kind,
               parent = excluded.parent,
               default_gst_treatment = excluded.default_gst_treatment,
               active = excluded.active",
            params![
                account.code.as_str(),
                account.name,
                enum_to_text(&account.kind)?,
                account.parent.as_ref().map(|p| p.as_str().to_string()),
                account
                    .default_gst_treatment
                    .as_ref()
                    .map(enum_to_text)
                    .transpose()?,
                account.active,
            ],
        )?;
        Ok(())
    }

    pub fn account(&self, code: &AccountCode) -> Result<Account> {
        self.conn
            .query_row(
                "SELECT code, name, kind, parent, default_gst_treatment, active
                 FROM accounts WHERE code = ?1",
                [code.as_str()],
                row_to_account,
            )
            .optional()?
            .ok_or_else(|| not_found("account", code))
    }

    pub fn accounts(&self, active_only: bool) -> Result<Vec<Account>> {
        let sql = if active_only {
            "SELECT code, name, kind, parent, default_gst_treatment, active
             FROM accounts WHERE active = 1 ORDER BY code"
        } else {
            "SELECT code, name, kind, parent, default_gst_treatment, active
             FROM accounts ORDER BY code"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let accounts = stmt
            .query_map([], row_to_account)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(accounts)
    }
}

fn row_to_account(row: &Row) -> rusqlite::Result<Account> {
    let code: String = row.get("code")?;
    let kind: String = row.get("kind")?;
    let parent: Option<String> = row.get("parent")?;
    let gst: Option<String> = row.get("default_gst_treatment")?;
    Ok(Account {
        code: code.parse().map_err(conv_err)?,
        name: row.get("name")?,
        kind: enum_from_text(&kind).map_err(conv_err)?,
        parent: parent.map(|p| p.parse()).transpose().map_err(conv_err)?,
        default_gst_treatment: gst
            .map(|t| enum_from_text(&t))
            .transpose()
            .map_err(conv_err)?,
        active: row.get("active")?,
    })
}

#[cfg(test)]
mod tests {
    use taxcore::{AccountKind, GstTreatment};

    use super::*;

    fn code(s: &str) -> AccountCode {
        AccountCode::new(s).unwrap()
    }

    #[test]
    fn accounts_upsert_and_round_trip() {
        let mut store = Store::open_in_memory().unwrap();

        let expenses = Account::new(code("6000-expenses"), "Expenses", AccountKind::Expense);
        store.upsert_account(&expenses).unwrap();

        let office = Account::new(code("6100-office"), "Office supplies", AccountKind::Expense)
            .with_gst(GstTreatment::Standard)
            .under(code("6000-expenses"));
        store.upsert_account(&office).unwrap();

        let loaded = store.account(&code("6100-office")).unwrap();
        assert_eq!(loaded.name, "Office supplies");
        assert_eq!(loaded.default_gst_treatment, Some(GstTreatment::Standard));
        assert_eq!(loaded.parent, Some(code("6000-expenses")));

        let mut retired = office.clone();
        retired.active = false;
        store.upsert_account(&retired).unwrap();

        assert_eq!(store.accounts(true).unwrap().len(), 1);
        assert_eq!(store.accounts(false).unwrap().len(), 2);
    }
}
