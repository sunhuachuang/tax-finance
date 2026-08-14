//! A minimal local dashboard over the tax engine, and the human confirmation
//! gate: approving or rejecting a draft happens here (or a future CLI), never
//! over MCP. Loopback only — this page shows financial data.

mod api;
mod demo;
mod http;

use std::net::TcpListener;
use std::path::PathBuf;

use anyhow::{Context as _, bail};
use taxstore::Store;

use crate::api::Ctx;

fn main() -> anyhow::Result<()> {
    let opts = parse_args()?;

    let (data_dir, rules_dir) = if opts.demo {
        let dir = std::env::temp_dir().join(format!("taxweb-demo-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let rules = opts
            .rules_dir
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rules"));
        (dir, rules)
    } else {
        let data = match opts.data_dir {
            Some(dir) => dir,
            None => {
                PathBuf::from(std::env::var("HOME").context("HOME is not set")?).join(".taxdata")
            }
        };
        let rules = opts.rules_dir.unwrap_or_else(|| data.join("rules"));
        (data, rules)
    };

    std::fs::create_dir_all(&data_dir)?;
    let mut store = Store::open(data_dir.join("ledger.db"))?;
    if opts.demo {
        demo::seed(&mut store, &data_dir, &rules_dir)?;
        eprintln!("taxweb: demo data seeded under {}", data_dir.display());
    }

    let mut ctx = Ctx {
        store,
        rules_dir,
    };

    let addr = format!("0.0.0.0:{}", opts.port);
    let listener = TcpListener::bind(&addr).with_context(|| format!("binding {addr}"))?;
    eprintln!("taxweb: open http://{addr}/  (data: {})", data_dir.display());

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("taxweb: accept failed: {e}");
                continue;
            }
        };
        let request = match http::read_request(&stream) {
            Ok(Some(req)) => req,
            Ok(None) => continue,
            Err(e) => {
                eprintln!("taxweb: bad request: {e}");
                continue;
            }
        };
        let (status, content_type, body) = api::route(&mut ctx, &request);
        if let Err(e) = http::respond(&mut stream, status, content_type, &body) {
            eprintln!("taxweb: write failed: {e}");
        }
    }
    Ok(())
}

struct Opts {
    port: u16,
    data_dir: Option<PathBuf>,
    rules_dir: Option<PathBuf>,
    demo: bool,
}

fn parse_args() -> anyhow::Result<Opts> {
    let mut opts = Opts {
        port: 5710,
        data_dir: None,
        rules_dir: None,
        demo: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                opts.port = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .context("--port needs a number")?;
            }
            "--data-dir" => opts.data_dir = Some(next_path(&arg, args.next())?),
            "--rules-dir" => opts.rules_dir = Some(next_path(&arg, args.next())?),
            "--demo" => opts.demo = true,
            "--help" | "-h" => {
                println!("usage: taxweb [--port N] [--data-dir DIR] [--rules-dir DIR] [--demo]");
                println!("  --port       listen port on 127.0.0.1 (default 5710)");
                println!("  --data-dir   ledger.db and stored documents (default ~/.taxdata)");
                println!("  --rules-dir  rule yaml files (default <data-dir>/rules)");
                println!("  --demo       seed a throwaway ledger in a temp dir and serve that");
                std::process::exit(0);
            }
            other => bail!("unknown argument {other}; see --help"),
        }
    }
    Ok(opts)
}

fn next_path(flag: &str, value: Option<String>) -> anyhow::Result<PathBuf> {
    match value {
        Some(v) => Ok(PathBuf::from(v)),
        None => bail!("{flag} needs a value"),
    }
}
