//! stdio MCP server. Newline-delimited JSON-RPC 2.0 on stdin/stdout.
//!
//! Usage:
//!   neural-memory-mcp --db <path> --as-of <rfc3339>
//!
//! `--as-of` is required and has no default. The server never reads a clock:
//! recency must be reproducible, and the reference instant is a recorded session
//! parameter rather than an ambient one. Supplying it at launch keeps the
//! decision with the operator.
//!
//! Diagnostics go to stderr only. Anything written to stdout that is not a
//! JSON-RPC message corrupts the stream.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use neural_memory_mcp::{handle_request, Session};
use neural_memory_store::Store;

fn main() -> ExitCode {
    let mut db: Option<PathBuf> = None;
    let mut as_of: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--db" => db = args.next().map(PathBuf::from),
            "--as-of" => as_of = args.next(),
            "--help" | "-h" => {
                eprintln!("neural-memory-mcp --db <path> --as-of <rfc3339>");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let (Some(db), Some(as_of)) = (db, as_of) else {
        eprintln!("both --db and --as-of are required (--as-of has no default: the server never reads a clock)");
        return ExitCode::from(2);
    };

    let store = match Store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open {}: {e}", db.display());
            return ExitCode::from(1);
        }
    };
    let session = Session { as_of };
    eprintln!(
        "neural-memory-mcp ready: db={} as_of={}",
        db.display(),
        session.as_of
    );

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("stdin: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                // Parse errors take id: null per JSON-RPC, since the id is
                // exactly what could not be read.
                let err = serde_json::json!({
                    "jsonrpc": "2.0", "id": null,
                    "error": {"code": -32700, "message": format!("parse error: {e}")}
                });
                let _ = writeln!(stdout, "{err}");
                let _ = stdout.flush();
                continue;
            }
        };
        if let Some(resp) = handle_request(&store, &session, &req) {
            if writeln!(stdout, "{resp}").is_err() {
                break;
            }
            let _ = stdout.flush();
        }
    }
    ExitCode::SUCCESS
}
