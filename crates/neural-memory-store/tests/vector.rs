//! Vector storage, search, and the index-sharing law.

use neural_memory_domain::*;
use neural_memory_store::*;

const NOW: &str = "2026-07-31T00:00:00Z";

fn profile(instruction: Option<&str>, dims: u32) -> EmbeddingProfileTerms {
    EmbeddingProfileTerms {
        model_family: "nomic-embed-text".into(),
        model_revision: "v1.5".into(),
        weight_sha256: vec!["aa".repeat(32)],
        tokenizer_sha256: vec!["bb".repeat(32)],
        dimensions: dims,
        pooling: Pooling::Mean,
        normalization: Normalization::L2,
        task_instruction: instruction.map(str::to_string),
    }
}

fn claim(s: &Store, text: &str) -> String {
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: text.into(),
            evidence_class: EvidenceClass::ExternalClaim,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![],
            harness_run_id: None,
        },
        occurred_at: None,
        recorded_at: None,
        derivation: None,
    };
    s.put_memory(WriteChannel::Operator, &w).unwrap().0
}

// ---------------------------------------------------------------------------
// The space identity
// ---------------------------------------------------------------------------

#[test]
fn changing_anything_substantive_forks_the_space() {
    let base = profile(Some("search_document: "), 768);
    let id = embedding_space_identity(&base);
    let mutate = |f: &dyn Fn(&mut EmbeddingProfileTerms)| {
        let mut p = base.clone();
        f(&mut p);
        embedding_space_identity(&p)
    };
    assert_ne!(id, mutate(&|p| p.model_revision = "v1.0".into()));
    assert_ne!(id, mutate(&|p| p.dimensions = 512));
    assert_ne!(id, mutate(&|p| p.pooling = Pooling::Cls));
    assert_ne!(id, mutate(&|p| p.normalization = Normalization::None));
    assert_ne!(id, mutate(&|p| p.task_instruction = None));
    assert_ne!(id, mutate(&|p| p.weight_sha256 = vec!["cc".repeat(32)]));
    assert_ne!(id, mutate(&|p| p.tokenizer_sha256 = vec!["cc".repeat(32)]));
    // Polarity, and the sort discipline: listing order is noise.
    let mut reordered = base.clone();
    reordered.weight_sha256 = vec!["aa".repeat(32)];
    assert_eq!(id, embedding_space_identity(&reordered));
}

#[test]
fn the_backend_is_absent_from_the_space_identity() {
    // Deliberate. Whether a CPU-derived and an NPU-derived vector share a space
    // is a question to MEASURE. Stamping the backend into the identity would
    // fork them by declaration and make the measurement unaskable.
    let p = profile(Some("search_document: "), 768);
    let json = serde_json::to_string(&p).unwrap();
    for banned in ["backend", "runtime", "device", "accelerator"] {
        assert!(
            !json.contains(banned),
            "{banned} must not be in the profile"
        );
    }
    // ...and registering the same profile under two backends yields ONE space.
    let s = Store::open_in_memory().unwrap();
    let a = s
        .register_embedding_profile(&p, "llama-cpp-cpu", 2048, NOW)
        .unwrap();
    let b = s
        .register_embedding_profile(&p, "ryzen-ai-npu", 2048, NOW)
        .unwrap();
    assert_eq!(a, b, "the backend must not fork the space");
}

#[test]
fn shares_index_is_decided_by_the_space_not_the_content() {
    let a = IndexEntryKey {
        content_sha256_hex: "11".repeat(32),
        embedding_space_identity: "aa".repeat(32),
    };
    let same_space_other_text = IndexEntryKey {
        content_sha256_hex: "22".repeat(32),
        embedding_space_identity: "aa".repeat(32),
    };
    let same_text_other_space = IndexEntryKey {
        content_sha256_hex: "11".repeat(32),
        embedding_space_identity: "bb".repeat(32),
    };
    assert!(shares_index(&a, &same_space_other_text));
    assert!(
        !shares_index(&a, &same_text_other_space),
        "identical text in a different space is never the same entry"
    );
}

// ---------------------------------------------------------------------------
// Storage and search
// ---------------------------------------------------------------------------

fn seeded() -> (Store, String, Vec<String>) {
    let s = Store::open_in_memory().unwrap();
    let pid = s
        .register_embedding_profile(&profile(Some("search_document: "), 4), "cpu", 512, NOW)
        .unwrap();
    let mut ds = Vec::new();
    for (text, v) in [
        ("thread scaling collapses at 24", [1.0f32, 0.0, 0.0, 0.0]),
        (
            "SMT contention on bandwidth-bound work",
            [0.9, 0.1, 0.0, 0.0],
        ),
        ("the carve-out ceiling is 7.9 GiB", [0.0, 1.0, 0.0, 0.0]),
    ] {
        let d = claim(&s, text);
        s.put_embedding(&pid, &d, &v, text, NOW).unwrap();
        ds.push(d);
    }
    (s, pid, ds)
}

#[test]
fn search_ranks_by_cosine_and_is_reproducible() {
    let (s, pid, ds) = seeded();
    let hits = s
        .vector_search(&pid, &[1.0, 0.0, 0.0, 0.0], 3, false)
        .unwrap();
    assert_eq!(hits[0].record_digest, ds[0]);
    assert_eq!(
        hits[1].record_digest, ds[1],
        "the near-parallel vector is second"
    );
    assert!(hits[0].similarity > hits[1].similarity);
    let again = s
        .vector_search(&pid, &[1.0, 0.0, 0.0, 0.0], 3, false)
        .unwrap();
    assert_eq!(
        hits.iter().map(|h| &h.record_digest).collect::<Vec<_>>(),
        again.iter().map(|h| &h.record_digest).collect::<Vec<_>>()
    );
}

#[test]
fn a_vector_of_the_wrong_length_is_refused() {
    // It would otherwise yield a plausible cosine against everything.
    let (s, pid, _) = seeded();
    let d = claim(&s, "x");
    assert!(matches!(
        s.put_embedding(&pid, &d, &[1.0, 0.0], "x", NOW),
        Err(VectorError::DimensionMismatch { .. })
    ));
    assert!(matches!(
        s.vector_search(&pid, &[1.0, 0.0], 3, false),
        Err(VectorError::DimensionMismatch { .. })
    ));
}

#[test]
fn a_non_finite_vector_is_refused() {
    let (s, pid, _) = seeded();
    let d = claim(&s, "x");
    assert!(matches!(
        s.put_embedding(&pid, &d, &[f32::NAN, 0.0, 0.0, 0.0], "x", NOW),
        Err(VectorError::NotFinite { .. })
    ));
}

#[test]
fn search_never_reaches_across_spaces() {
    // The correctness property the whole module exists for. Cosine between
    // vectors from two spaces returns a plausible number; nothing errors and
    // the results are simply wrong.
    let (s, pid_a, ds) = seeded();
    let pid_b = s
        .register_embedding_profile(&profile(Some("classify: "), 4), "cpu", 512, NOW)
        .unwrap();
    assert_ne!(pid_a, pid_b);
    s.put_embedding(&pid_b, &ds[2], &[1.0, 0.0, 0.0, 0.0], "x", NOW)
        .unwrap();

    // In space B only the one record exists, and it is the perfect match there.
    let b = s
        .vector_search(&pid_b, &[1.0, 0.0, 0.0, 0.0], 10, false)
        .unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].record_digest, ds[2]);

    // In space A the same query returns A's three, and NOT via B's vector.
    let a = s
        .vector_search(&pid_a, &[1.0, 0.0, 0.0, 0.0], 10, false)
        .unwrap();
    assert_eq!(a.len(), 3);
    assert_eq!(
        a[0].record_digest, ds[0],
        "space A's own best match, not B's"
    );
}

#[test]
fn retired_records_are_excluded_unless_asked_for() {
    let (s, pid, ds) = seeded();
    s.supersede(&ds[0], &ds[1], NOW).unwrap();
    let hits = s
        .vector_search(&pid, &[1.0, 0.0, 0.0, 0.0], 10, false)
        .unwrap();
    assert!(hits.iter().all(|h| h.record_digest != ds[0]));
    let with = s
        .vector_search(&pid, &[1.0, 0.0, 0.0, 0.0], 10, true)
        .unwrap();
    assert!(with.iter().any(|h| h.record_digest == ds[0]));
}

#[test]
fn coverage_is_reported_so_a_partial_index_is_visible() {
    // A semantic branch over half the corpus is not a semantic branch, and the
    // caller should be able to tell.
    let (s, pid, _) = seeded();
    claim(&s, "a record with no vector");
    let (embedded, total) = s.embedding_coverage(&pid).unwrap();
    assert_eq!((embedded, total), (3, 4));
}

#[test]
fn brute_force_scan_is_fast_enough_to_need_no_index() {
    // The review's claim, measured rather than assumed: if a scan stops being
    // fast enough, this test says so instead of the day being a surprise.
    let s = Store::open_in_memory().unwrap();
    let dims = 768;
    let pid = s
        .register_embedding_profile(&profile(None, dims), "cpu", 2048, NOW)
        .unwrap();
    let n = 5000;
    for i in 0..n {
        let d = claim(&s, &format!("record number {i}"));
        let v: Vec<f32> = (0..dims).map(|k| ((i + k) % 97) as f32 / 97.0).collect();
        s.put_embedding(&pid, &d, &v, "x", NOW).unwrap();
    }
    let q: Vec<f32> = (0..dims).map(|k| (k % 97) as f32 / 97.0).collect();
    let t = std::time::Instant::now();
    let hits = s.vector_search(&pid, &q, 10, false).unwrap();
    let ms = t.elapsed().as_millis();
    assert_eq!(hits.len(), 10);
    assert!(
        ms < 2000,
        "{n}x{dims} brute-force scan took {ms}ms; at that point an index starts \
         to be worth its dependency"
    );
    println!("  {n} x {dims} scan: {ms}ms");
}

#[test]
fn coverage_cannot_exceed_one_hundred_percent() {
    // Caught on real data: retired records ARE embedded, so that
    // include_retired search works, but counting them against a live-only
    // denominator reported 26/24.
    let (s, pid, ds) = seeded();
    s.supersede(&ds[0], &ds[1], NOW).unwrap();
    let (embedded, total) = s.embedding_coverage(&pid).unwrap();
    assert!(
        embedded <= total,
        "coverage {embedded}/{total} exceeds 100%"
    );
    assert_eq!(
        (embedded, total),
        (2, 2),
        "the retired record leaves both sides"
    );

    // Polarity: it is still searchable when explicitly asked for, so excluding
    // it from coverage is not the same as failing to embed it.
    let with = s
        .vector_search(&pid, &[1.0, 0.0, 0.0, 0.0], 10, true)
        .unwrap();
    assert!(with.iter().any(|h| h.record_digest == ds[0]));
}
