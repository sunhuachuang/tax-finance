//! Routes. Reads mirror what MCP exposes; the two confirmation routes
//! (approve/reject) exist *only* here, behind a click on localhost — this
//! page is the human gate.

use std::path::PathBuf;

use chrono::{NaiveDate, Utc};
use serde_json::{Value, json};
use taxcore::{DocumentStatus, EntryId, EntryStatus, TaxYear};
use taxingest::{propose_matches, review_queue};
use taxrules::RuleSet;
use taxstore::Store;

use crate::http::Request;

pub struct Ctx {
    pub store: Store,
    pub rules_dir: PathBuf,
}

impl Ctx {
    fn rules_for(&self, year: TaxYear) -> Result<RuleSet, String> {
        RuleSet::for_year(&self.rules_dir, "NZ", year).map_err(|e| e.to_string())
    }
}

pub fn route(ctx: &mut Ctx, req: &Request) -> (u16, &'static str, Vec<u8>) {
    let result = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => {
            return (
                200,
                "text/html; charset=utf-8",
                include_bytes!("index.html").to_vec(),
            );
        }
        ("GET", "/api/overview") => overview(ctx),
        ("GET", "/api/gst") => gst(ctx, req),
        ("GET", "/api/ir3") => ir3(ctx, req),
        ("POST", path) if path.starts_with("/api/entries/") => entry_action(ctx, path),
        ("GET", _) => return (404, "application/json", br#"{"error":"not found"}"#.to_vec()),
        _ => {
            return (
                405,
                "application/json",
                br#"{"error":"method not allowed"}"#.to_vec(),
            );
        }
    };

    match result {
        Ok(value) => (200, "application/json", value.to_string().into_bytes()),
        Err(message) => (
            400,
            "application/json",
            json!({ "error": message }).to_string().into_bytes(),
        ),
    }
}

fn overview(ctx: &mut Ctx) -> Result<Value, String> {
    let queue = review_queue(&ctx.store).map_err(err)?;
    let mut documents = Vec::new();
    for status in [
        DocumentStatus::PendingExtraction,
        DocumentStatus::Extracted,
        DocumentStatus::NeedsReview,
        DocumentStatus::Ignored,
    ] {
        documents.extend(ctx.store.documents_with_status(status).map_err(err)?);
    }
    Ok(json!({
        "review_documents": to_value(&queue.documents)?,
        "review_drafts": to_value(&queue.drafts)?,
        "documents": to_value(&documents)?,
        "posted_entries": to_value(&ctx.store.entries_with_status(EntryStatus::Posted).map_err(err)?)?,
        "unreconciled_bank": to_value(&ctx.store.unreconciled_bank_txns().map_err(err)?)?,
        "match_candidates": to_value(&propose_matches(&ctx.store, None).map_err(err)?)?,
        "accounts": to_value(&ctx.store.accounts(false).map_err(err)?)?,
    }))
}

fn gst(ctx: &mut Ctx, req: &Request) -> Result<Value, String> {
    let date = match req.query.get("date") {
        Some(s) => s
            .parse::<NaiveDate>()
            .map_err(|_| format!("{s} is not a YYYY-MM-DD date"))?,
        None => Utc::now().date_naive(),
    };
    let rules = ctx.rules_for(TaxYear::containing(date))?;
    let frequency = match req.query.get("frequency").map(String::as_str) {
        None | Some("") => rules.gst.default_frequency(),
        Some(id) => rules
            .gst
            .frequency(id)
            .ok_or_else(|| format!("unknown filing frequency {id}"))?,
    }
    .frequency()
    .map_err(err)?;
    let period = frequency.period_containing(date);

    let period_year = TaxYear::containing(period.end);
    let rules = if period_year == TaxYear::containing(date) {
        rules
    } else {
        ctx.rules_for(period_year)?
    };
    to_value(&taxreturn::gst101(&ctx.store, &rules, period).map_err(err)?)
}

fn ir3(ctx: &mut Ctx, req: &Request) -> Result<Value, String> {
    let year: TaxYear = req
        .query
        .get("year")
        .ok_or("missing query parameter year")?
        .parse()
        .map_err(err)?;
    let rules = ctx.rules_for(year)?;
    to_value(&taxreturn::ir3(&ctx.store, &rules, year).map_err(err)?)
}

/// POST /api/entries/{id}/approve | /api/entries/{id}/reject — the human gate.
fn entry_action(ctx: &mut Ctx, path: &str) -> Result<Value, String> {
    let rest = path.trim_start_matches("/api/entries/");
    let (id, action) = rest
        .split_once('/')
        .ok_or("expected /api/entries/{id}/{approve|reject}")?;
    let id: EntryId = id.parse().map_err(err)?;
    match action {
        "approve" => taxingest::approve_draft(&mut ctx.store, id).map_err(err)?,
        "reject" => taxingest::reject_draft(&mut ctx.store, id).map_err(err)?,
        other => return Err(format!("unknown action {other}")),
    }
    Ok(json!({ "ok": true }))
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(err)
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
