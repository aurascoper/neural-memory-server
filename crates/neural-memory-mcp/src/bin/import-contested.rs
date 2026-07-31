//! Import the contested corpus. Operator channel.
use std::path::PathBuf;
use std::process::ExitCode;

use neural_memory_mcp::corpus_contested::import_contested;
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
        eprintln!("usage: neural-memory-import-contested --db <path>");
        return ExitCode::from(2);
    };
    let store = match Store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open {}: {e}", db.display());
            return ExitCode::from(1);
        }
    };
    match import_contested(&store) {
        Ok(m) => {
            println!("{m}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("import failed: {e}");
            ExitCode::from(1)
        }
    }
}
