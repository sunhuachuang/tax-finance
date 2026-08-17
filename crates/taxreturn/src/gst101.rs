use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::Serialize;
use taxcore::{Currency, GstPeriod, Money, ReturnLine, ReturnRunId, Rounding};
use taxrules::RuleSet;
use taxstore::Store;

use crate::scan::{build_line, scan};
use crate::{Result, ReturnError};

/// One computed GST return. Boxes follow the GST101 form; see the crate docs
/// for how boxes 8 and 12 differ from the printed shortcut.
#[derive(Debug, Serialize)]
pub struct Gst101 {
    pub run: ReturnRunId,
    pub period: GstPeriod,
    pub due: NaiveDate,
    /// The rule file (year + version) these figures were computed under.
    pub rules_year: String,
    pub rules_version: u32,
    pub lines: Vec<ReturnLine>,
    /// Box 7 × 3/23 — the form's shortcut for GST on sales, for comparison
    /// with the invoice-exact box 8.
    pub formula_gst_on_sales: Money,
    /// Box 11 × 3/23, for comparison with the invoice-exact box 12.
    pub formula_gst_on_purchases: Money,
    pub warnings: Vec<String>,
}

impl Gst101 {
    pub fn line(&self, code: &str) -> Option<&ReturnLine> {
        self.lines.iter().find(|l| l.code == code)
    }

    /// Box 15: positive is GST to pay, negative a refund due.
    pub fn gst_to_pay(&self) -> Money {
        self.line("gst101.box15")
            .map(|l| l.amount)
            .unwrap_or(Money::zero(Currency::NZD))
    }
}

/// Compute the GST return for one filing period from the posted ledger.
pub fn gst101(store: &Store, rules: &RuleSet, period: GstPeriod) -> Result<Gst101> {
    let rules_year = rules.meta.tax_year()?;
    if !rules_year.contains(period.end) {
        return Err(ReturnError::WrongRulesYear {
            rules: rules_year.label(),
            wanted: format!("period ending {}", period.end),
        });
    }

    let scanned = scan(store, period.start, period.end)?;
    let totals = &scanned.totals;
    let mut warnings = scanned.warnings;
    let mut sources = BTreeMap::new();

    let box5 = build_line(
        store,
        "gst101.box5",
        "Total sales and income (including GST and zero-rated supplies)",
        totals,
        |t| Ok(t.sales_incl),
        &mut sources,
    )?;
    let box6 = build_line(
        store,
        "gst101.box6",
        "Zero-rated supplies included in Box 5",
        totals,
        |t| Ok(t.zero_rated),
        &mut sources,
    )?;
    let box7 = build_line(
        store,
        "gst101.box7",
        "Sales and income subject to GST (Box 5 minus Box 6)",
        totals,
        |t| t.sales_incl.sub(t.zero_rated),
        &mut sources,
    )?;
    let box8 = build_line(
        store,
        "gst101.box8",
        "GST collected on sales (invoice-exact)",
        totals,
        |t| Ok(t.output_gst),
        &mut sources,
    )?;
    let box9 = adjustments_placeholder("gst101.box9", "Adjustments");
    let box10 = build_line(
        store,
        "gst101.box10",
        "Total GST collected (Box 8 plus Box 9)",
        totals,
        |t| Ok(t.output_gst),
        &mut sources,
    )?;
    let box11 = build_line(
        store,
        "gst101.box11",
        "Total purchases and expenses including GST",
        totals,
        |t| Ok(t.purchases_incl),
        &mut sources,
    )?;
    let box12 = build_line(
        store,
        "gst101.box12",
        "GST credit on purchases (invoice-exact)",
        totals,
        |t| Ok(t.input_gst),
        &mut sources,
    )?;
    let box13 = adjustments_placeholder("gst101.box13", "Credit adjustments");
    let box14 = build_line(
        store,
        "gst101.box14",
        "Total credit (Box 12 plus Box 13)",
        totals,
        |t| Ok(t.input_gst),
        &mut sources,
    )?;
    let box15 = build_line(
        store,
        "gst101.box15",
        "GST to pay (positive) or refund (negative): Box 10 minus Box 14",
        totals,
        |t| t.output_gst.sub(t.input_gst),
        &mut sources,
    )?;

    let rate = rules.gst_rate();
    let formula_gst_on_sales = rate.extract_from_inclusive(box7.amount, Rounding::HalfUp)?;
    let formula_gst_on_purchases = rate.extract_from_inclusive(box11.amount, Rounding::HalfUp)?;
    for (label, exact, formula) in [
        ("sales", box8.amount, formula_gst_on_sales),
        ("purchases", box12.amount, formula_gst_on_purchases),
    ] {
        let diff = exact.sub(formula)?;
        if !diff.is_zero() {
            warnings.push(format!(
                "invoice-exact GST on {label} is {exact}, the form's 3/23 shortcut gives \
                 {formula} (difference {diff}) — per-invoice rounding explains a few cents, \
                 anything larger deserves a look",
            ));
        }
    }

    Ok(Gst101 {
        run: ReturnRunId::new(),
        period,
        due: rules.gst.due_date(period.end),
        rules_year: rules_year.label(),
        rules_version: rules.meta.version,
        lines: vec![
            box5, box6, box7, box8, box9, box10, box11, box12, box13, box14, box15,
        ],
        formula_gst_on_sales,
        formula_gst_on_purchases,
        warnings,
    })
}

/// Boxes 9 and 13 (calculation-sheet adjustments) are not modelled yet. A zero
/// line with no contributions verifies trivially and keeps the box numbering
/// honest until they are.
fn adjustments_placeholder(code: &str, label: &str) -> ReturnLine {
    ReturnLine::new(
        code,
        format!("{label} (not modelled yet; enter manually if any)"),
        Money::zero(Currency::NZD),
        Vec::new(),
    )
}

#[cfg(test)]
mod tests {
    use taxcore::{EntrySource, GstFrequency};

    use crate::testutil::{d, park_draft, post_export, post_purchase, post_sale, rules, store_with_chart};

    use super::*;

    fn may_2025_period() -> GstPeriod {
        GstFrequency::two_monthly_ending_march().period_containing(d(2025, 4, 15))
    }

    #[test]
    fn a_quiet_period_files_zeros() {
        let store = store_with_chart();
        let gst = gst101(&store, &rules(), may_2025_period()).unwrap();
        assert!(gst.gst_to_pay().is_zero());
        assert_eq!(gst.due, d(2025, 6, 28));
        assert!(gst.warnings.is_empty());
    }

    #[test]
    fn the_boxes_add_up_and_trace_back() {
        let mut store = store_with_chart();
        // Sale $230.00 (GST $30.00), export $500.00, purchase $115.00 (GST $15.00).
        post_sale(&mut store, d(2025, 4, 10), 23000, 3000);
        post_export(&mut store, d(2025, 4, 20), 50000);
        post_purchase(&mut store, d(2025, 5, 2), 11500, 1500);
        // Drafts stay invisible however large.
        park_draft(&mut store, d(2025, 4, 11));

        let gst = gst101(&store, &rules(), may_2025_period()).unwrap();

        let amount = |code: &str| gst.line(code).unwrap().amount;
        assert_eq!(amount("gst101.box5"), Money::nzd(73000));
        assert_eq!(amount("gst101.box6"), Money::nzd(50000));
        assert_eq!(amount("gst101.box7"), Money::nzd(23000));
        assert_eq!(amount("gst101.box8"), Money::nzd(3000));
        assert_eq!(amount("gst101.box11"), Money::nzd(11500));
        assert_eq!(amount("gst101.box12"), Money::nzd(1500));
        assert_eq!(gst.gst_to_pay(), Money::nzd(1500));

        // Round numbers agree with the form's shortcut exactly.
        assert_eq!(gst.formula_gst_on_sales, Money::nzd(3000));
        assert!(gst.warnings.is_empty());

        // Every line still verifies and box 5 knows which entries built it.
        for line in &gst.lines {
            line.verify().unwrap();
        }
        let box5 = gst.line("gst101.box5").unwrap();
        assert_eq!(box5.contributions.len(), 2);
        assert!(box5.contributions.iter().all(|c| !c.sources.is_empty()));
    }

    #[test]
    fn a_reversed_entry_nets_out_but_stays_visible() {
        let mut store = store_with_chart();
        post_purchase(&mut store, d(2025, 4, 5), 11500, 1500);
        let wrong = post_purchase(&mut store, d(2025, 4, 6), 99900, 13030);
        store
            .reverse_entry(wrong, d(2025, 4, 7), EntrySource::Human, None)
            .unwrap();

        let gst = gst101(&store, &rules(), may_2025_period()).unwrap();
        let box11 = gst.line("gst101.box11").unwrap();
        assert_eq!(box11.amount, Money::nzd(11500));
        // The mistake and its correction both appear in the audit trail.
        assert_eq!(box11.contributions.len(), 3);
    }

    /// The cardinal rule: a return already filed must recompute to the same
    /// numbers years later. Correcting a mistake in a later period is normal —
    /// it must land in the period the correction was made, and leave the
    /// earlier return exactly where it was.
    #[test]
    fn a_later_reversal_does_not_rewrite_an_already_filed_period() {
        let mut store = store_with_chart();
        let wrong = post_purchase(&mut store, d(2025, 4, 5), 11500, 1500);

        let april_may = may_2025_period();
        let filed = gst101(&store, &rules(), april_may).unwrap();
        assert_eq!(filed.line("gst101.box11").unwrap().amount, Money::nzd(11500));

        // Noticed two months later and reversed then, not backdated.
        store
            .reverse_entry(wrong, d(2025, 7, 10), EntrySource::Human, None)
            .unwrap();

        let refiled = gst101(&store, &rules(), april_may).unwrap();
        assert_eq!(
            refiled.line("gst101.box11").unwrap().amount,
            filed.line("gst101.box11").unwrap().amount,
            "reversing in July must not move the April-May figures"
        );

        // The correction shows up in the period it was actually made.
        let june_july = GstFrequency::two_monthly_ending_march().period_containing(d(2025, 7, 10));
        let later = gst101(&store, &rules(), june_july).unwrap();
        assert_eq!(later.line("gst101.box11").unwrap().amount, Money::nzd(-11500));
        assert_eq!(later.line("gst101.box11").unwrap().contributions.len(), 1);
    }

    #[test]
    fn periods_outside_the_rule_file_are_refused() {
        let store = store_with_chart();
        let period = GstFrequency::two_monthly_ending_march().period_containing(d(2030, 6, 1));
        let err = gst101(&store, &rules(), period).unwrap_err();
        assert!(matches!(err, ReturnError::WrongRulesYear { .. }));
    }
}
