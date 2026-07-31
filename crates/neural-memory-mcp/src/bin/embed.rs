//! Backfill vectors for records that lack them. **Operator channel.**
//!
//! Usage:
//!   neural-memory-embed --db P --url http://127.0.0.1:8082 \
//!       --model-sha256 H --revision v1.5 --dims 768 [--limit N]
//!
//! Registering the space and embedding into it are one action deliberately: a
//! vector whose space was never declared cannot be compared to anything, and
//! declaring a space nothing was embedded into is an empty promise.

use std::path::PathBuf;
use std::process::ExitCode;

use neural_memory_domain::{EmbeddingProfileTerms, Normalization, Pooling};
use neural_memory_mcp::embed::Embedder;
use neural_memory_store::Store;

fn main() -> ExitCode {
    let mut m = std::collections::HashMap::new();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        if let Some(k) = argv[i].strip_prefix("--") {
            m.insert(k.to_string(), argv.get(i + 1).cloned().unwrap_or_default());
            i += 2;
        } else {
            eprintln!("unexpected argument: {}", argv[i]);
            return ExitCode::from(2);
        }
    }
    let need = |k: &str| m.get(k).cloned();
    let (Some(db), Some(url), Some(sha), Some(rev), Some(dims)) = (
        need("db"),
        need("url"),
        need("model-sha256"),
        need("revision"),
        need("dims"),
    ) else {
        eprintln!("required: --db --url --model-sha256 --revision --dims");
        return ExitCode::from(2);
    };
    let dims: u32 = match dims.parse() {
        Ok(d) => d,
        Err(_) => {
            eprintln!("--dims must be an integer");
            return ExitCode::from(2);
        }
    };
    let at = m
        .get("at")
        .cloned()
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into());
    let limit: usize = m
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);

    let store = match Store::open(&PathBuf::from(&db)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open {db}: {e}");
            return ExitCode::from(1);
        }
    };

    let profile = EmbeddingProfileTerms {
        model_family: m
            .get("family")
            .cloned()
            .unwrap_or_else(|| "nomic-embed-text".into()),
        model_revision: rev,
        weight_sha256: vec![sha],
        tokenizer_sha256: vec![m
            .get("tokenizer-sha256")
            .cloned()
            .unwrap_or_else(|| "0".repeat(64))],
        dimensions: dims,
        pooling: Pooling::Mean,
        normalization: Normalization::L2,
        task_instruction: Some("search_document: ".into()),
    };
    let pid = match store.register_embedding_profile(&profile, "llama-cpp-cpu", 2048, &at) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("register: {e}");
            return ExitCode::from(1);
        }
    };
    println!("profile {pid}");

    let e = Embedder {
        url,
        profile_identity: pid.clone(),
        document_prefix: "search_document: ".into(),
        query_prefix: "search_query: ".into(),
        dimensions: dims as usize,
    };
    if let Err(err) = e.probe() {
        eprintln!("embedder unreachable: {err}");
        return ExitCode::from(1);
    }

    // Only records missing a vector IN THIS SPACE. A record embedded in another
    // space is not embedded here, and treating it as done would leave a hole
    // that only shows up as quietly missing search results.
    let todo: Vec<(String, String)> = {
        let mut st = store
            .conn
            .prepare(
                "SELECT m.record_digest, m.claim FROM memories m
             WHERE NOT EXISTS (SELECT 1 FROM embeddings e
                 WHERE e.record_digest = m.record_digest AND e.profile_identity = ?1)
             ORDER BY m.recorded_seq",
            )
            .unwrap();
        st.query_map([&pid], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|x| x.unwrap())
            .take(limit)
            .collect()
    };
    println!("{} records to embed", todo.len());

    let (mut ok, mut failed) = (0usize, 0usize);
    for (digest, claim) in &todo {
        match e.embed_document(claim) {
            Ok((v, text)) => match store.put_embedding(&pid, digest, &v, &text, &at) {
                Ok(()) => ok += 1,
                Err(err) => {
                    eprintln!("  store {}: {err}", &digest[..12]);
                    failed += 1;
                }
            },
            Err(err) => {
                eprintln!("  embed {}: {err}", &digest[..12]);
                failed += 1;
            }
        }
    }
    let (cov, total) = store.embedding_coverage(&pid).unwrap_or((0, 0));
    println!("embedded {ok}, failed {failed}; coverage {cov}/{total}");
    if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
