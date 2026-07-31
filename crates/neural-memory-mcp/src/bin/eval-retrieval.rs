//! Branch ablation against the store directly. **Operator channel.**
//!
//! Drives `Store::recall` with exact `RecallOptions` rather than going through
//! the MCP surface, because that surface hardcodes `entities: true`. A first
//! attempt at this ablation ran through MCP and post-filtered records whose
//! branch list was exactly `["entity"]` -- which leaves records found by BOTH
//! semantic and entity in place, still carrying their entity score. The
//! "semantic only" arm was therefore contaminated by entity scoring, and the
//! numbers it produced were not measuring what they claimed.

use std::path::PathBuf;
use std::process::ExitCode;

use neural_memory_mcp::embed::Embedder;
use neural_memory_store::{retrieval::SemanticQuery, RecallOptions, Store};

fn main() -> ExitCode {
    let mut m = std::collections::HashMap::new();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        if let Some(k) = argv[i].strip_prefix("--") {
            m.insert(k.to_string(), argv.get(i + 1).cloned().unwrap_or_default());
            i += 2;
        } else {
            i += 1;
        }
    }
    let (Some(db), Some(url), Some(pid), Some(query)) =
        (m.get("db"), m.get("url"), m.get("profile"), m.get("query"))
    else {
        eprintln!(
            "required: --db --url --profile --query [--semantic 0|1] [--entities 0|1] [--hops N]"
        );
        return ExitCode::from(2);
    };
    let want = |k: &str, d: bool| m.get(k).map(|v| v == "1").unwrap_or(d);
    let hops: u32 = m.get("hops").and_then(|s| s.parse().ok()).unwrap_or(0);

    let store = match Store::open(&PathBuf::from(db)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let dims: i64 = store
        .conn
        .query_row(
            "SELECT dim FROM embedding_profiles WHERE identity = ?1",
            [pid],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let vec_holder;
    let semantic = if want("semantic", false) {
        let e = Embedder {
            url: url.clone(),
            profile_identity: pid.clone(),
            document_prefix: "search_document: ".into(),
            query_prefix: "search_query: ".into(),
            dimensions: dims as usize,
        };
        match e.embed_query(query) {
            Ok(v) => {
                vec_holder = v;
                Some(SemanticQuery {
                    profile_identity: pid,
                    vector: &vec_holder,
                })
            }
            Err(err) => {
                eprintln!("embed: {err}");
                return ExitCode::from(1);
            }
        }
    } else {
        None
    };

    let opt = RecallOptions {
        query,
        entities: want("entities", false),
        semantic,
        as_of: "2026-07-31T14:00:00Z",
        limit: 10,
        max_hops: hops,
        include_retired: false,
    };
    match store.recall(&opt) {
        Ok(r) => {
            for h in &r.hits {
                println!("{}", h.record_digest);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}
