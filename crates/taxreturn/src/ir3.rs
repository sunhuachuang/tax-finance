use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::Serialize;
use taxcore::{ReturnLine, ReturnRunId, TaxYear};
use taxrules::{IncomeTaxBreakdown, RuleSet};
use taxstore::Store;

use crate::scan::{build_line, scan};
use crate::{Result, ReturnError};

/// The sole-trader IR3 picture for one tax year: business income, deductible
/// expenses and net profit from the ledger (all GST-exclusive, as income tax
/// requires of a registered person), with the tax on it computed band by band
/// from the rule file.
#[derive(Debug, Serialize)]
pub struct Ir3Summary {
    pub run: ReturnRunId,
    pub year: TaxYear,
    pub rules_version: u32,
    /// `ir3.income`, `ir3.expenses`, `ir3.net_profit` — each verified and
    /// carrying the entries behind it.
    pub lines: Vec<ReturnLine>,
    pub tax: IncomeTaxBreakdown,
    pub self_filed_due: NaiveDate,
    /// Honesty about scope: what these figures do not yet include.
    pub notes: Vec<String>,
}

impl Ir3Summary {
    pub fn line(&self, code: &str) -> Option<&ReturnLine> {
        self.lines.iter().find(|l| l.code == code)
    }
}

pub fn ir3(store: &Store, rules: &RuleSet, year: TaxYear) -> Result<Ir3Summary> {
    let rules_year = rules.meta.tax_year()?;
    if rules_year != year {
        return Err(ReturnError::WrongRulesYear {
            rules: rules_year.label(),
            wanted: year.label(),
        });
    }

    let scanned = scan(store, year.start(), year.end())?;
    let totals = &scanned.totals;
    let mut sources = BTreeMap::new();

    let income = build_line(
        store,
        "ir3.income",
        "Business income (GST-exclusive)",
        totals,
        |t| Ok(t.income_excl),
        &mut sources,
    )?;
    let expenses = build_line(
        store,
        "ir3.expenses",
        "Deductible expenses (GST-exclusive)",
        totals,
        |t| Ok(t.expenses_excl),
        &mut sources,
    )?;
    let net_profit = build_line(
        store,
        "ir3.net_profit",
        "Net profit before adjustments",
        totals,
        |t| t.income_excl.sub(t.expenses_excl),
        &mut sources,
    )?;

    let tax = rules.income_tax.tax_on(net_profit.amount)?;

    let mut notes = scanned.warnings;
    notes.push(
        "statutory deduction adjustments are not applied yet — entertainment (50%), home \
         office and vehicle apportionments must be entered as manual entries or reviewed \
         separately"
            .to_string(),
    );
    if rules.provisional_tax.applies_to(tax.total)? {
        notes.push(format!(
            "residual income tax {} exceeds the provisional tax threshold {} — provisional \
             tax likely applies next year",
            tax.total,
            rules.provisional_tax.threshold()?
        ));
    }

    Ok(Ir3Summary {
        run: ReturnRunId::new(),
        year,
        rules_version: rules.meta.version,
        lines: vec![income, expenses, net_profit],
        tax,
        self_filed_due: rules.income_tax.self_filed_due(year),
        notes,
    })
}

#[cfg(test)]
mod tests {
    use taxcore::Money;

    use crate::testutil::{d, post_export, post_purchase, post_sale, post_wages, rules, store_with_chart};

    use super::*;

    #[test]
    fn the_year_aggregates_gst_exclusive_and_taxes_the_profit() {
        let mut store = store_with_chart();
        // In NZD cents: sale 230.00 (GST 30), export 500.00 (zero-rated),
        // purchase 115.00 (GST 15), wages 200.00 (no GST).
        post_sale(&mut store, d(2025, 6, 10), 23000, 3000);
        post_export(&mut store, d(2025, 9, 1), 50000);
        post_purchase(&mut store, d(2026, 2, 14), 11500, 1500);
        post_wages(&mut store, d(2026, 3, 1), 20000);

        let summary = ir3(&store, &rules(), TaxYear(2026)).unwrap();

        let amount = |code: &str| summary.line(code).unwrap().amount;
        assert_eq!(amount("ir3.income"), Money::nzd(70000)); // 200 + 500
        assert_eq!(amount("ir3.expenses"), Money::nzd(30000)); // 100 + 200
        assert_eq!(amount("ir3.net_profit"), Money::nzd(40000));

        // 400.00 all in the 10.5% band → 42.00.
        assert_eq!(summary.tax.total, Money::nzd(4200));
        assert_eq!(summary.tax.bands.len(), 1);
        assert_eq!(summary.self_filed_due, d(2026, 7, 7));
        for line in &summary.lines {
            line.verify().unwrap();
        }
    }

    #[test]
    fn entries_outside_the_year_do_not_leak_in() {
        let mut store = store_with_chart();
        post_sale(&mut store, d(2025, 3, 31), 23000, 3000); // 2024-25
        post_sale(&mut store, d(2025, 4, 1), 46000, 6000); // 2025-26

        let summary = ir3(&store, &rules(), TaxYear(2026)).unwrap();
        assert_eq!(
            summary.line("ir3.income").unwrap().amount,
            Money::nzd(40000)
        );
    }

    #[test]
    fn the_wrong_rule_file_is_refused() {
        let store = store_with_chart();
        let err = ir3(&store, &rules(), TaxYear(2027)).unwrap_err();
        assert!(matches!(err, ReturnError::WrongRulesYear { .. }));
    }
}
