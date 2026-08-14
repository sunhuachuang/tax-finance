//! MCP server over the tax engine (stdio transport, line-delimited JSON-RPC).
//!
//! This binary is deliberately thin: every capability lives in the library
//! crates and this layer only maps tools onto them. The security posture from
//! the roadmap is enforced by *omission* — read tools are open; write tools
//! (`ingest_document`, `record_reading`, `propose_draft`, `import_bank_rows`)
//! can only create pending records; and there is no tool at all for approving,
//! posting, reversing or voiding an entry, or for forcing a document status.
//! Confirmation stays with a human, outside this process.

mod server;
mod tools;

use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context as _, bail};
use taxstore::Store;

use crate::server::Server;
use crate::tools::Context;

fn main() -> anyhow::Result<()> {
    let opts = parse_args()?;
    std::fs::create_dir_all(&opts.data_dir)
        .with_context(|| format!("creating data dir {}", opts.data_dir.display()))?;
    let store = Store::open(opts.data_dir.join("ledger.db"))?;

    eprintln!(
        "taxmcp: data dir {}, rules dir {}",
        opts.data_dir.display(),
        opts.rules_dir.display()
    );

    let mut server = Server::new(Context {
        store,
        data_dir: opts.data_dir,
        rules_dir: opts.rules_dir,
    });

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.context("reading stdin")?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(&line) {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

struct Opts {
    data_dir: PathBuf,
    rules_dir: PathBuf,
}

fn parse_args() -> anyhow::Result<Opts> {
    let mut data_dir: Option<PathBuf> = None;
    let mut rules_dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => data_dir = Some(expect_value(&arg, args.next())?),
            "--rules-dir" => rules_dir = Some(expect_value(&arg, args.next())?),
            "--help" | "-h" => {
                println!("usage: taxmcp [--data-dir DIR] [--rules-dir DIR]");
                println!("  --data-dir   ledger.db and stored documents (default ~/.taxdata)");
                println!("  --rules-dir  rule yaml files (default <data-dir>/rules)");
                std::process::exit(0);
            }
            other => bail!("unknown argument {other}; see --help"),
        }
    }
    let data_dir = match data_dir {
        Some(dir) => dir,
        None => PathBuf::from(std::env::var("HOME").context("HOME is not set")?).join(".taxdata"),
    };
    let rules_dir = rules_dir.unwrap_or_else(|| data_dir.join("rules"));
    Ok(Opts {
        data_dir,
        rules_dir,
    })
}

fn expect_value(flag: &str, value: Option<String>) -> anyhow::Result<PathBuf> {
    match value {
        Some(v) => Ok(PathBuf::from(v)),
        None => bail!("{flag} needs a value"),
    }
}
