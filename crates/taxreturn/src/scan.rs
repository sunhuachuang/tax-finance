use std::collections::BTreeMap;

use chrono::NaiveDate;
use taxcore::{
    Account, AccountKind, Contribution, Currency, EntryId, GstTreatment, Money, ReturnLine,
    SourceRef,
};
use taxstore::Store;

use crate::{Result, ReturnError};

/// One posted entry's contribution to every figure a return might want. All
/// sums are signed, so a reversal pair nets to zero without special casing.
pub(crate) struct EntryTotals {
    pub entry: EntryId,
    pub narration: String,
    /// Income postings with return-total treatments (standard + zero-rated),
    /// GST-inclusive, sign-flipped so revenue is positive.
    pub sales_incl: Money,
    /// The zero-rated part of `sales_incl`.
    pub zero_rated: Money,
    /// GST recorded on income postings, sign-flipped.
    pub output_gst: Money,
    /// Standard-rated expense postings, GST-inclusive.
    pub purchases_incl: Money,
    /// GST recorded on standard-rated expense postings.
    pub input_gst: Money,
    /// All income postings net of their GST content — what income tax sees.
    pub income_excl: Money,
    /// All expense postings net of their GST content.
    pub expenses_excl: Money,
}

pub(crate) struct Scan {
    pub totals: Vec<EntryTotals>,
    pub warnings: Vec<String>,
}

/// Walk the posted ledger over a period once, classifying every posting by its
/// account's kind and its GST treatment.
pub(crate) fn scan(store: &Store, from: NaiveDate, to: NaiveDate) -> Result<Scan> {
    let chart: BTreeMap<String, Account> = store
        .accounts(false)?
        .into_iter()
        .map(|a| (a.code.to_string(), a))
        .collect();

    let mut totals = Vec::new();
    let mut warnings = Vec::new();
    let zero = Money::zero(Currency::NZD);

    for entry in store.posted_entries_between(from, to)? {
        let mut t = EntryTotals {
            entry: entry.id,
            narration: entry.narration.clone(),
            sales_incl: zero,
            zero_rated: zero,
            output_gst: zero,
            purchases_incl: zero,
            input_gst: zero,
            income_excl: zero,
            expenses_excl: zero,
        };

        for posting in &entry.postings {
            let account = chart.get(posting.account.as_str()).ok_or_else(|| {
                ReturnError::UnknownAccount {
                    code: posting.account.to_string(),
                    entry: entry.id.to_string(),
                }
            })?;
            let gst = posting.gst_amount.unwrap_or(zero);
            let excl = posting.amount.sub(gst)?;

            match account.kind {
                AccountKind::Income => {
                    // Income sits on the credit side, so flip signs to report
                    // revenue as positive.
                    if posting.gst_treatment.included_in_return_totals() {
                        t.sales_incl = t.sales_incl.sub(posting.amount)?;
                    }
                    if posting.gst_treatment == GstTreatment::ZeroRated {
                        t.zero_rated = t.zero_rated.sub(posting.amount)?;
                    }
                    t.output_gst = t.output_gst.sub(gst)?;
                    t.income_excl = t.income_excl.sub(excl)?;
                }
                AccountKind::Expense => {
                    if posting.gst_treatment == GstTreatment::Standard {
                        t.purchases_incl = t.purchases_incl.add(posting.amount)?;
                        if posting.gst_amount.is_none() {
                            // Never invent a credit: a standard-rated purchase
                            // with no recorded GST claims nothing and gets
                            // flagged instead.
                            warnings.push(format!(
                                "entry {} ({}): standard-rated posting to {} has no recorded \
                                 GST content; no input credit was claimed for it",
                                entry.id, entry.narration, posting.account
                            ));
                        }
                        t.input_gst = t.input_gst.add(gst)?;
                    }
                    t.expenses_excl = t.expenses_excl.add(excl)?;
                }
                AccountKind::Asset | AccountKind::Liability | AccountKind::Equity => {}
            }
        }

        totals.push(t);
    }

    Ok(Scan { totals, warnings })
}

/// Build a verified [`ReturnLine`] from one figure per entry. Zero
/// contributions are dropped; what remains must sum to the line exactly.
pub(crate) fn build_line(
    store: &Store,
    code: &str,
    label: &str,
    totals: &[EntryTotals],
    pick: impl Fn(&EntryTotals) -> taxcore::Result<Money>,
    sources: &mut BTreeMap<EntryId, Vec<SourceRef>>,
) -> Result<ReturnLine> {
    let mut amount = Money::zero(Currency::NZD);
    let mut contributions = Vec::new();
    for t in totals {
        let part = pick(t)?;
        if part.is_zero() {
            continue;
        }
        amount = amount.add(part)?;
        if !sources.contains_key(&t.entry) {
            sources.insert(t.entry, store.provenance(t.entry)?.sources);
        }
        contributions.push(Contribution {
            entry: t.entry,
            amount: part,
            narration: t.narration.clone(),
            sources: sources[&t.entry].clone(),
        });
    }
    let line = ReturnLine::new(code, label, amount, contributions);
    line.verify()?;
    Ok(line)
}
