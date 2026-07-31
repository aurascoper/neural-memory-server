//! Ingest a declarative evidence document. **Operator channel.**
//!
//! Usage:
//!   neural-memory-ingest --db <path> --file <doc.toml> [--dry-run]
//!
//! `--dry-run` parses, resolves every alias and checks every reference against a
//! scratch database without writing. Evidence you have to delete afterwards was
//! never append-only, so checking before committing is the normal path.

use std::path::PathBuf;
use std::process::ExitCode;

use neural_memory_store::{ingest::ingest, Store};

fn main() -> ExitCode {
    let (mut db, mut file, mut dry) = (None, None, false);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--db" => db = args.next().map(PathBuf::from),
            "--file" => file = args.next().map(PathBuf::from),
            "--dry-run" => dry = true,
            other => {
                eprintln!("unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
    }
    let (Some(db), Some(file)) = (db, file) else {
        eprintln!("usage: neural-memory-ingest --db <path> --file <doc.toml> [--dry-run]");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            return ExitCode::from(1);
        }
    };
    let store = match Store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open {}: {e}", db.display());
            return ExitCode::from(1);
        }
    };
    match ingest(&store, &text, dry) {
        Ok(r) => {
            println!("{}: {r}", file.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}: {e}", file.display());
            ExitCode::from(1)
        }
    }
}
