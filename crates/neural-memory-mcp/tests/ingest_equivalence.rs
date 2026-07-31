//! Does the declarative format actually express the hand-written corpus?
//!
//! The claim is exact: ingesting `corpus/contested.toml` must produce the same
//! record digests as running `corpus_contested.rs`. Comparing counts, or
//! checking a couple of claims are present, would let a real divergence through
//! — a different locator, a dropped observation link, a changed evidence class
//! all leave the counts identical and the seals different.
//!
//! This lives here rather than in the store crate because it needs both
//! importers, and store cannot depend on mcp.

use neural_memory_mcp::corpus_contested::import_contested;
use neural_memory_store::{ingest::ingest, Store};

fn digests(s: &Store) -> Vec<(String, String)> {
    let mut st = s
        .conn
        .prepare("SELECT record_digest, claim FROM memories ORDER BY record_digest")
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|x| x.unwrap())
        .collect()
}

fn edges(s: &Store) -> Vec<(String, String, String)> {
    let mut st = s
        .conn
        .prepare(
            "SELECT src_digest, dst_digest, edge_kind FROM provenance_edges
             ORDER BY src_digest, dst_digest, edge_kind",
        )
        .unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|x| x.unwrap())
        .collect()
}

#[test]
fn the_toml_corpus_seals_identically_to_the_rust_one() {
    let from_rust = Store::open_in_memory().unwrap();
    import_contested(&from_rust).expect("rust importer");

    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/contested.toml"
    ))
    .unwrap();
    let from_toml = Store::open_in_memory().unwrap();
    ingest(&from_toml, &text, false).expect("toml importer");

    let (r, t) = (digests(&from_rust), digests(&from_toml));
    assert_eq!(r.len(), 7);
    // Compared element by element so a mismatch names the claim that diverged
    // rather than just reporting that two vectors differ.
    for ((rd, rc), (td, tc)) in r.iter().zip(t.iter()) {
        assert_eq!(rd, td, "seal differs for:\n  rust: {rc}\n  toml: {tc}");
        assert_eq!(rc, tc);
    }
    assert_eq!(r.len(), t.len(), "claim counts differ");

    // Observations and edges too: a corpus is its wiring as much as its text.
    let obs = |s: &Store| -> Vec<String> {
        let mut st = s
            .conn
            .prepare("SELECT identity FROM observations ORDER BY identity")
            .unwrap();
        st.query_map([], |x| x.get(0))
            .unwrap()
            .map(|x| x.unwrap())
            .collect()
    };
    assert_eq!(obs(&from_rust), obs(&from_toml), "observation seals differ");
    assert_eq!(
        edges(&from_rust),
        edges(&from_toml),
        "provenance wiring differs"
    );
}

#[test]
fn a_changed_claim_text_would_be_caught() {
    // Polarity: the equivalence test above must be capable of failing. If the
    // seals were insensitive to content it would pass against anything.
    let a = Store::open_in_memory().unwrap();
    import_contested(&a).unwrap();
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/contested.toml"
    ))
    .unwrap()
    .replace(
        "-27.4 percent against the 8-thread",
        "-27.4 percent against the 9-thread",
    );
    let b = Store::open_in_memory().unwrap();
    // The derivation still recomputes (the transform is unchanged), so this is
    // caught by the seal rather than by the arithmetic check.
    ingest(&b, &text, false).unwrap();
    assert_ne!(digests(&a), digests(&b));
}
