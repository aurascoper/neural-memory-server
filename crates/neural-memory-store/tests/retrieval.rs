//! Retrieval: lexical, provenance, the merge, and the retirement filter.

use neural_memory_domain::*;
use neural_memory_store::*;

const NOW: &str = "2026-07-30T12:00:00Z";

fn store() -> Store {
    Store::open_in_memory().unwrap()
}

/// Write a claim through the operator door as a low-trust external claim, which
/// needs no artifact or derivation, so these tests exercise retrieval only.
fn claim(s: &Store, text: &str, occurred_at: Option<&str>) -> String {
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: text.into(),
            evidence_class: EvidenceClass::ExternalClaim,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![],
            harness_run_id: None,
        },
        occurred_at,
        recorded_at: occurred_at,
        derivation: None,
    };
    s.put_memory(WriteChannel::Operator, &w).unwrap().0
}

fn opts<'a>(query: &'a str) -> RecallOptions<'a> {
    RecallOptions {
        entities: true,
        semantic: None,
        query,
        as_of: NOW,
        limit: 20,
        max_hops: 1,
        include_retired: false,
    }
}

// ---------------------------------------------------------------------------
// Lexical
// ---------------------------------------------------------------------------

#[test]
fn lexical_recall_finds_by_content_and_ranks_by_relevance() {
    let s = store();
    claim(
        &s,
        "Gemma diverged from the CPU reference at greedy step 52",
        None,
    );
    claim(
        &s,
        "Qwen showed no divergence within the measured step budget",
        None,
    );
    claim(
        &s,
        "The carve-out ceiling is 7.9 GiB of device-local memory",
        None,
    );

    let r = s.recall(&opts("divergence")).unwrap();
    assert!(!r.hits.is_empty());
    assert!(
        r.hits[0].claim.contains("diverg"),
        "top hit should be about divergence, got {:?}",
        r.hits[0].claim
    );
    assert!(r.hits.iter().all(|h| h.branches.contains(&Branch::Lexical)));

    // Polarity: an unrelated query must not return them.
    assert!(s
        .recall(&opts("thermal throttling"))
        .unwrap()
        .hits
        .is_empty());
}

#[test]
fn a_query_with_fts_operators_is_not_a_syntax_error() {
    // FTS5 treats -, *, :, ^ and NEAR as operators. A raw user string is both a
    // syntax hazard and an injection surface into the match grammar.
    let s = store();
    claim(&s, "carve-out ceiling is 7.9 GiB", None);

    for hostile in [
        "carve-out",
        "\"unterminated",
        "NEAR(a b)",
        "* OR *",
        "a:b^2",
        "-ceiling",
        "",
    ] {
        let r = s.recall(&opts(hostile));
        assert!(r.is_ok(), "query {hostile:?} must not error: {:?}", r.err());
    }

    // ...and the sanitiser must not have destroyed matching in the process.
    assert!(!s.recall(&opts("carve-out")).unwrap().hits.is_empty());
}

// ---------------------------------------------------------------------------
// The weights
// ---------------------------------------------------------------------------

#[test]
fn recency_cannot_outrank_relevance() {
    // THE invariant behind re-deriving the weights instead of renormalising
    // claude-mind's. Rescaling 0.45 -> 1.0 would put recency at 0.44 against
    // lexical 0.33, and this test would fail: a barely-relevant record from
    // today would beat a highly-relevant one from a year ago. In a store of
    // dated measurements that is close to the worst possible ranking.
    let s = store();
    let old_relevant = claim(
        &s,
        "logit divergence divergence divergence measured against the CPU reference",
        Some("2025-07-30T00:00:00Z"), // a year before as_of
    );
    let new_marginal = claim(
        &s,
        "divergence appears once in this otherwise unrelated note about packaging",
        Some("2026-07-30T00:00:00Z"), // today
    );

    let r = s.recall(&opts("divergence")).unwrap();
    assert_eq!(
        r.hits[0].record_digest, old_relevant,
        "the older, far more relevant record must win; recency is a tiebreaker"
    );
    assert!(r.hits.iter().any(|h| h.record_digest == new_marginal));

    // The newer record does score higher on recency alone -- so the ordering
    // above is the weighting doing its job, not recency being ignored.
    let newer = r
        .hits
        .iter()
        .find(|h| h.record_digest == new_marginal)
        .unwrap();
    let older = r
        .hits
        .iter()
        .find(|h| h.record_digest == old_relevant)
        .unwrap();
    assert!(newer.recency_score > older.recency_score);
}

#[test]
fn a_missing_occurred_at_is_neutral_not_infinitely_old() {
    let s = store();
    claim(&s, "divergence with no date recorded", None);
    let r = s.recall(&opts("divergence")).unwrap();
    assert_eq!(
        r.hits[0].recency_score, 0.5,
        "unknown is not the same as ancient"
    );
}

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

#[test]
fn provenance_expansion_surfaces_records_the_query_never_mentions() {
    let s = store();
    let hit = claim(
        &s,
        "Vulkan exceeded the preregistered logit tolerance",
        None,
    );
    // Deliberately shares NO vocabulary with the query -- if it did, the
    // lexical branch would find it and the polarity check below would pass for
    // the wrong reason.
    let support = claim(
        &s,
        "0.5 was fixed on principle before any capture ran",
        None,
    );
    s.add_edge(&hit, &support, "derivedFrom", NOW).unwrap();

    let r = s.recall(&opts("Vulkan tolerance")).unwrap();
    let found = r.hits.iter().find(|h| h.record_digest == support).unwrap();
    assert!(found.branches.contains(&Branch::Provenance));
    assert_eq!(found.graph_distance, Some(1));
    assert!(r.counts.provenance >= 1);

    // Polarity: with expansion disabled it is not reachable.
    let mut o = opts("Vulkan tolerance");
    o.max_hops = 0;
    let r0 = s.recall(&o).unwrap();
    assert!(!r0.hits.iter().any(|h| h.record_digest == support));
}

#[test]
fn a_record_found_by_both_branches_reports_both() {
    let s = store();
    let a = claim(&s, "divergence exceeded tolerance", None);
    let b = claim(&s, "divergence measured at step 52", None);
    s.add_edge(&a, &b, "supports", NOW).unwrap();

    let r = s.recall(&opts("divergence")).unwrap();
    let both = r.hits.iter().find(|h| h.record_digest == b).unwrap();
    assert!(both.branches.contains(&Branch::Lexical));
    assert!(both.branches.contains(&Branch::Provenance));
    assert_eq!(r.counts.unique, 2, "attribution must not double-count");
}

#[test]
fn traversal_terminates_on_a_cycle() {
    // supersedes chains and derivedFrom links form cycles readily. An unguarded
    // recursive CTE does not merely return duplicates here -- it does not
    // terminate, and the test would hang rather than fail.
    let s = store();
    let a = claim(&s, "alpha", None);
    let b = claim(&s, "beta", None);
    let c = claim(&s, "gamma", None);
    s.add_edge(&a, &b, "derivedFrom", NOW).unwrap();
    s.add_edge(&b, &c, "derivedFrom", NOW).unwrap();
    s.add_edge(&c, &a, "derivedFrom", NOW).unwrap(); // closes the loop

    let walk = s.traverse(&a, Direction::Forward, 10).unwrap();
    assert_eq!(walk.len(), 3, "each node once, despite the cycle: {walk:?}");
    assert_eq!(walk.iter().find(|(d, _)| d == &a).unwrap().1, 0);
    assert_eq!(walk.iter().find(|(d, _)| d == &b).unwrap().1, 1);
    assert_eq!(walk.iter().find(|(d, _)| d == &c).unwrap().1, 2);
}

#[test]
fn traversal_respects_direction() {
    let s = store();
    let derived = claim(&s, "the 7.27x speedup", None);
    let input = claim(&s, "pp512 vulkan 287.17", None);
    s.add_edge(&derived, &input, "derivedFrom", NOW).unwrap();

    // Forward from the derived record reaches its input.
    let fwd = s.traverse(&derived, Direction::Forward, 3).unwrap();
    assert!(fwd.iter().any(|(d, _)| d == &input));

    // Forward from the input does NOT reach the derived record...
    let fwd_from_input = s.traverse(&input, Direction::Forward, 3).unwrap();
    assert!(!fwd_from_input.iter().any(|(d, _)| d == &derived));

    // ...but backward does. "What rests on this?" is a different question from
    // "what does this rest on?", and conflating them invents support.
    let back = s.traverse(&input, Direction::Backward, 3).unwrap();
    assert!(back.iter().any(|(d, _)| d == &derived));
}

#[test]
fn max_hops_bounds_the_walk() {
    let s = store();
    let a = claim(&s, "a", None);
    let b = claim(&s, "b", None);
    let c = claim(&s, "c", None);
    s.add_edge(&a, &b, "derivedFrom", NOW).unwrap();
    s.add_edge(&b, &c, "derivedFrom", NOW).unwrap();

    assert_eq!(s.traverse(&a, Direction::Forward, 1).unwrap().len(), 2);
    assert_eq!(s.traverse(&a, Direction::Forward, 2).unwrap().len(), 3);
}

#[test]
fn trace_provenance_reports_edge_kinds_and_depth() {
    let s = store();
    let top = claim(&s, "Vulkan requires a separate numerical contract", None);
    let mid = claim(&s, "max logit delta was 4.3362", None);
    let base = claim(&s, "the CPU reference execution", None);
    s.add_edge(&top, &mid, "derivedFrom", NOW).unwrap();
    s.add_edge(&mid, &base, "derivedFrom", NOW).unwrap();

    let trace = s.trace_provenance(&top, 3).unwrap();
    assert_eq!(trace.len(), 2);
    assert_eq!(trace[0].record_digest, mid);
    assert_eq!(trace[0].hops, 1);
    assert!(trace[0].edge_kinds.contains(&"derivedFrom".to_string()));
    assert_eq!(trace[1].record_digest, base);
    assert_eq!(trace[1].hops, 2);
}

// ---------------------------------------------------------------------------
// Retirement
// ---------------------------------------------------------------------------

#[test]
fn retired_records_are_withheld_by_default_and_reported_as_withheld() {
    // The §1 case: the battery 14B figures are retired in favour of the AC ones.
    // Grep over markdown returns both with equal standing; this must not.
    let s = store();
    let old = claim(&s, "Qwen3 14B pp512 is 159.96 tokens per second", None);
    let new = claim(&s, "Qwen3 14B pp512 is 147.91 tokens per second", None);
    s.supersede(&old, &new, NOW).unwrap();

    let r = s.recall(&opts("14B pp512")).unwrap();
    assert!(
        r.hits.iter().all(|h| h.record_digest != old),
        "a retired claim must not come back with equal standing to its replacement"
    );
    assert!(r.hits.iter().any(|h| h.record_digest == new));
    assert!(
        r.withheld_retired.contains(&old),
        "withholding must be visible: 'nothing' and 'something, retired' differ"
    );
}

#[test]
fn retired_records_are_still_retrievable_when_asked_for() {
    // Polarity, and the H5 invariant: superseded records remain historically
    // reconstructable while being excluded from current-truth retrieval.
    let s = store();
    let old = claim(&s, "Qwen3 14B pp512 is 159.96 tokens per second", None);
    let new = claim(&s, "Qwen3 14B pp512 is 147.91 tokens per second", None);
    s.supersede(&old, &new, NOW).unwrap();

    let mut o = opts("14B pp512");
    o.include_retired = true;
    let r = s.recall(&o).unwrap();

    let hit = r.hits.iter().find(|h| h.record_digest == old).unwrap();
    assert!(hit.superseded);
    assert_eq!(
        hit.superseded_by.as_deref(),
        Some(new.as_str()),
        "and it says what replaced it, so the correction is followable"
    );
    assert!(r.withheld_retired.is_empty());
}

#[test]
fn a_retired_record_is_not_reachable_by_provenance_either() {
    // Otherwise the graph branch would quietly reintroduce exactly what the
    // lexical filter excluded.
    let s = store();
    let live = claim(&s, "divergence exceeded tolerance", None);
    let old = claim(&s, "an earlier and now retired supporting note", None);
    let new = claim(&s, "its replacement", None);
    s.add_edge(&live, &old, "derivedFrom", NOW).unwrap();
    s.supersede(&old, &new, NOW).unwrap();

    let r = s.recall(&opts("divergence")).unwrap();
    assert!(!r.hits.iter().any(|h| h.record_digest == old));
    assert!(r.withheld_retired.contains(&old));
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn recall_is_reproducible_for_the_same_inputs() {
    let s = store();
    for i in 0..8 {
        claim(&s, &format!("divergence observation number {i}"), None);
    }
    let a = s.recall(&opts("divergence")).unwrap();
    let b = s.recall(&opts("divergence")).unwrap();
    let ids = |r: &RecallResult| -> Vec<String> {
        r.hits.iter().map(|h| h.record_digest.clone()).collect()
    };
    assert_eq!(ids(&a), ids(&b), "ties must break deterministically");
}

#[test]
fn as_of_drives_recency_rather_than_the_wall_clock() {
    // A store that reads now() is not reproducible, and has acquired the clock
    // dependency the mobile core's doctrine forbids.
    let s = store();
    claim(&s, "divergence measured", Some("2026-01-01T00:00:00Z"));

    let mut early = opts("divergence");
    early.as_of = "2026-01-02T00:00:00Z";
    let mut late = opts("divergence");
    late.as_of = "2027-01-01T00:00:00Z";

    let e = s.recall(&early).unwrap().hits[0].recency_score;
    let l = s.recall(&late).unwrap().hits[0].recency_score;
    assert!(
        e > l,
        "the same record is fresher as of an earlier as_of ({e} vs {l})"
    );
}

#[test]
fn the_limit_is_applied_after_ranking_not_before() {
    let s = store();
    for i in 0..10 {
        claim(&s, &format!("divergence note {i}"), None);
    }
    let best = claim(
        &s,
        "divergence divergence divergence divergence the most relevant one",
        None,
    );
    let mut o = opts("divergence");
    o.limit = 3;
    let r = s.recall(&o).unwrap();
    assert_eq!(r.hits.len(), 3);
    assert_eq!(
        r.hits[0].record_digest, best,
        "truncating before ranking would drop the best hit"
    );
    assert_eq!(
        r.counts.unique, 11,
        "counts describe the candidate set, not the page"
    );
}

// ---------------------------------------------------------------------------
// Conflict surfacing (added after H6 arm (c))
// ---------------------------------------------------------------------------

#[test]
fn a_contradiction_is_pushed_onto_both_hits() {
    // Found by H6 arm (c): retirement was pushed to the caller on every hit,
    // contradiction had to be pulled via get_record, and nothing told the
    // caller to pull. An agent that only called recall could not see that two
    // claims disagreed. Both models tested duly failed to notice.
    let s = store();
    let a = claim(
        &s,
        "Vulkan is blocked because the policy has no backend dimension",
        None,
    );
    let b = claim(
        &s,
        "Vulkan is blocked by the measurement corpus, not the schema",
        None,
    );
    s.add_edge(&a, &b, "contradicts", NOW).unwrap();

    let r = s.recall(&opts("Vulkan blocked")).unwrap();
    let ha = r.hits.iter().find(|h| h.record_digest == a).unwrap();
    let hb = r.hits.iter().find(|h| h.record_digest == b).unwrap();

    // Symmetric: the edge was written a -> b, but the conflict holds both ways.
    assert_eq!(ha.conflicts_with, vec![b.clone()]);
    assert_eq!(
        hb.conflicts_with,
        vec![a.clone()],
        "the side the edge was written FROM must not be the only one told"
    );

    // And the pair is surfaced at the top level, so ignoring it takes effort.
    let expect = if a < b {
        (a.clone(), b.clone())
    } else {
        (b.clone(), a.clone())
    };
    assert_eq!(r.conflicting_pairs, vec![expect]);
}

#[test]
fn a_pair_is_only_reported_when_both_sides_are_present() {
    // Otherwise the caller is told about a conflict it cannot inspect.
    let s = store();
    let a = claim(&s, "divergence exceeded the tolerance", None);
    let b = claim(&s, "an entirely unrelated note about packaging", None);
    s.add_edge(&a, &b, "contradicts", NOW).unwrap();

    let mut o = opts("divergence tolerance");
    o.max_hops = 0; // keep b out of the result set
    let r = s.recall(&o).unwrap();
    assert!(r.hits.iter().any(|h| h.record_digest == a));
    assert!(!r.hits.iter().any(|h| h.record_digest == b));
    assert!(
        r.conflicting_pairs.is_empty(),
        "one-sided conflicts are not pairs"
    );

    // The hit still knows what it conflicts with, so the caller can go look.
    let ha = r.hits.iter().find(|h| h.record_digest == a).unwrap();
    assert_eq!(ha.conflicts_with, vec![b]);
}

#[test]
fn records_with_no_conflict_report_none() {
    // Polarity: a field that were always populated would be no signal at all.
    let s = store();
    claim(&s, "divergence exceeded the tolerance", None);
    let r = s.recall(&opts("divergence")).unwrap();
    assert!(r.hits.iter().all(|h| h.conflicts_with.is_empty()));
    assert!(r.conflicting_pairs.is_empty());
}

#[test]
fn a_supersedes_edge_is_not_reported_as_a_conflict() {
    // Retirement already has its own signal. Reporting it twice, under a name
    // that means "we cannot tell which is right", would misdescribe a case
    // where the store knows exactly which is right.
    let s = store();
    let old = claim(&s, "14B pp512 is 159.96", None);
    let new = claim(&s, "14B pp512 is 147.91", None);
    s.supersede(&old, &new, NOW).unwrap();

    let mut o = opts("14B pp512");
    o.include_retired = true;
    let r = s.recall(&o).unwrap();
    assert!(r.conflicting_pairs.is_empty());
    assert!(r.hits.iter().all(|h| h.conflicts_with.is_empty()));
    // ...and the retirement signal is still there.
    assert!(r
        .hits
        .iter()
        .any(|h| h.superseded && h.superseded_by.is_some()));
}
