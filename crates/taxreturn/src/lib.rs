//! Return generation: the posted ledger in, filing figures out.
//!
//! Everything here is deterministic aggregation — no model is consulted and
//! nothing is estimated. Every figure is a [`taxcore::ReturnLine`] whose
//! contributions name the entries (and through their provenance, the
//! documents) behind it, and every line is `verify()`-ed before it leaves:
//! a number that cannot be explained is not shipped.
//!
//! One deliberate departure from the printed GST101 form: boxes 8 and 12 are
//! the *sum of the GST recorded on each posting* — invoice-exact, fully
//! attributable — rather than the form's 3/23 shortcut applied to the box 7
//! and 11 aggregates. The shortcut values are computed alongside for
//! comparison and any difference is surfaced as a warning for the filer.

mod error;
mod gst101;
mod ir3;
mod scan;

pub use error::{Result, ReturnError};
pub use gst101::{Gst101, gst101};
pub use ir3::{Ir3Summary, ir3};

#[cfg(test)]
mod testutil {
    use std::path::PathBuf;

    use chrono::NaiveDate;
    use taxcore::{
        Account, AccountCode, AccountKind, EntryBuilder, EntrySource, EntryStatus, GstTreatment,
        Money, Posting, Provenance, SourceRef, TaxYear,
    };
    use taxrules::RuleSet;
    use taxstore::Store;

    pub fn code(s: &str) -> AccountCode {
        AccountCode::new(s).unwrap()
    }

    pub fn rules() -> RuleSet {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules");
        RuleSet::for_year(&dir, "NZ", TaxYear(2026)).unwrap()
    }

    pub fn store_with_chart() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        for account in [
            Account::new(code("4000-sales"), "Sales", AccountKind::Income)
                .with_gst(GstTreatment::Standard),
            Account::new(code("4100-exports"), "Export sales", AccountKind::Income)
                .with_gst(GstTreatment::ZeroRated),
            Account::new(code("6100-office"), "Office supplies", AccountKind::Expense)
                .with_gst(GstTreatment::Standard),
            Account::new(code("6200-wages"), "Wages", AccountKind::Expense)
                .with_gst(GstTreatment::NotSubject),
            Account::new(code("1010-bank"), "Bank", AccountKind::Asset),
        ] {
            store.upsert_account(&account).unwrap();
        }
        store
    }

    pub fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn post(store: &mut Store, entry: taxcore::Entry) -> taxcore::EntryId {
        let id = entry.id;
        let provenance = Provenance::new(id, vec![SourceRef::Manual]);
        store.insert_entry(&entry, &provenance).unwrap();
        store.post_entry(id).unwrap();
        id
    }

    /// A standard-rated sale of `incl` cents (GST `gst` cents), banked.
    pub fn post_sale(store: &mut Store, date: NaiveDate, incl: i64, gst: i64) -> taxcore::EntryId {
        let entry = EntryBuilder::new(date, "Consulting invoice", EntrySource::Human)
            .posting(
                Posting::new(code("4000-sales"), Money::nzd(-incl), GstTreatment::Standard)
                    .with_gst_amount(Money::nzd(-gst)),
            )
            .debit(code("1010-bank"), Money::nzd(incl), GstTreatment::NotSubject)
            .build()
            .unwrap();
        post(store, entry)
    }

    /// A zero-rated export sale, banked.
    pub fn post_export(store: &mut Store, date: NaiveDate, amount: i64) -> taxcore::EntryId {
        let entry = EntryBuilder::new(date, "Export invoice", EntrySource::Human)
            .credit(code("4100-exports"), Money::nzd(amount), GstTreatment::ZeroRated)
            .debit(code("1010-bank"), Money::nzd(amount), GstTreatment::NotSubject)
            .build()
            .unwrap();
        post(store, entry)
    }

    /// A standard-rated purchase of `incl` cents (GST `gst` cents), paid from
    /// the bank.
    pub fn post_purchase(
        store: &mut Store,
        date: NaiveDate,
        incl: i64,
        gst: i64,
    ) -> taxcore::EntryId {
        let entry = EntryBuilder::new(date, "Officeworks paper", EntrySource::Human)
            .posting(
                Posting::new(code("6100-office"), Money::nzd(incl), GstTreatment::Standard)
                    .with_gst_amount(Money::nzd(gst)),
            )
            .credit(code("1010-bank"), Money::nzd(incl), GstTreatment::NotSubject)
            .build()
            .unwrap();
        post(store, entry)
    }

    /// Wages: an expense with no GST content.
    pub fn post_wages(store: &mut Store, date: NaiveDate, amount: i64) -> taxcore::EntryId {
        let entry = EntryBuilder::new(date, "Wages", EntrySource::Human)
            .debit(code("6200-wages"), Money::nzd(amount), GstTreatment::NotSubject)
            .credit(code("1010-bank"), Money::nzd(amount), GstTreatment::NotSubject)
            .build()
            .unwrap();
        post(store, entry)
    }

    /// Insert a draft that must never show up in any return.
    pub fn park_draft(store: &mut Store, date: NaiveDate) {
        let entry = EntryBuilder::new(date, "Unconfirmed proposal", EntrySource::Human)
            .posting(
                Posting::new(code("6100-office"), Money::nzd(77700), GstTreatment::Standard)
                    .with_gst_amount(Money::nzd(10135)),
            )
            .credit(code("1010-bank"), Money::nzd(77700), GstTreatment::NotSubject)
            .build()
            .unwrap();
        assert_eq!(entry.status, EntryStatus::Draft);
        let provenance = Provenance::new(entry.id, vec![SourceRef::Manual]);
        store.insert_entry(&entry, &provenance).unwrap();
    }
}
