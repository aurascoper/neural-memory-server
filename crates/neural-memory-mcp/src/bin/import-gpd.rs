//! Thin wrapper around `neural_memory_mcp::corpus::import_gpd`.
//!
//! The import logic lives in the library so it is testable without spawning a
//! process -- an importer that can only be exercised by running it is an
//! importer whose idempotency claim is untested.

use std::path::PathBuf;
use std::process::ExitCode;

use neural_memory_mcp::corpus::import_gpd;
use neural_memory_store::Store;

fn main() -> ExitCode {
    let mut db: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--db" {
            db = args.next().map(PathBuf::from);
        }
    }
    let Some(db) = db else {
        eprintln!("usage: neural-memory-import-gpd --db <path>");
        return ExitCode::from(2);
    };
    let store = match Store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open {}: {e}", db.display());
            return ExitCode::from(1);
        }
    };
    match import_gpd(&store) {
        Ok(msg) => {
            println!("{msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("import failed: {e}");
            ExitCode::from(1)
        }
    }
}
