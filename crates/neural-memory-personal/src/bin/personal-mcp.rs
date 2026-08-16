use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use neural_memory_personal::{personal_mcp::handle_request, PersonalStore};
use serde_json::json;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != "--db" {
        eprintln!("neural-memory-personal-mcp --db <personal.db>");
        return ExitCode::from(2);
    }
    let path = PathBuf::from(&args[1]);
    let mut store = match PersonalStore::open(&path) {
        Ok(store) => store,
        Err(error) => {
            eprintln!("cannot open {}: {error}", path.display());
            return ExitCode::from(1);
        }
    };
    eprintln!("neural-memory-personal-mcp ready: db={}", path.display());
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("stdin: {error}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                let response = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("parse error: {error}")}});
                if writeln!(stdout, "{response}").is_err() {
                    break;
                }
                let _ = stdout.flush();
                continue;
            }
        };
        if let Some(response) = handle_request(&mut store, &request) {
            if writeln!(stdout, "{response}").is_err() {
                break;
            }
            let _ = stdout.flush();
        }
    }
    ExitCode::SUCCESS
}
