//! Append-only assembly invariants, and the H1' cost comparison.
//!
//! H1': over a session of N queries, an append-only assembler produces fewer
//! total prefill tokens than a rebuild-per-query assembler, and the gap widens
//! with N. This is a design-validation hypothesis, not a justification for the
//! store — the store is justified on provenance, not latency.

use neural_memory_domain::*;

fn d(n: u8) -> String {
    format!("{:0>64}", format!("{n:x}"))
}

const CFG: AssemblerConfig = AssemblerConfig {
    context_budget_tokens: 8192,
};

#[test]
fn a_record_already_in_the_prefix_is_never_re_emitted() {
    let mut prefix = SessionPrefix::new();

    let t1 = plan_append(&prefix, &[d(1), d(2), d(3)]);
    assert_eq!(t1.append, vec![d(1), d(2), d(3)]);
    assert!(t1.suppressed.is_empty());
    prefix.apply(&t1);

    // Turn 2 retrieves one seen record and one new one.
    let t2 = plan_append(&prefix, &[d(2), d(4)]);
    assert_eq!(t2.append, vec![d(4)], "only the unseen record is appended");
    assert_eq!(
        t2.suppressed,
        vec![d(2)],
        "the seen one is reported, not silently dropped"
    );
    prefix.apply(&t2);

    assert_eq!(prefix.emitted, vec![d(1), d(2), d(3), d(4)]);
}

#[test]
fn appending_never_reorders_what_is_already_there() {
    let mut prefix = SessionPrefix::new();
    prefix.apply(&plan_append(&prefix.clone(), &[d(9), d(1), d(5)]));
    let before = prefix.emitted.clone();
    let id_before = session_prefix_identity(&prefix);

    prefix.apply(&plan_append(&prefix.clone(), &[d(2)]));

    assert_eq!(
        &prefix.emitted[..before.len()],
        &before[..],
        "the existing prefix must be a byte-identical head of the new one"
    );
    assert_ne!(id_before, session_prefix_identity(&prefix));
}

#[test]
fn append_order_does_not_depend_on_retrieval_order() {
    let prefix = SessionPrefix::new();
    let forward = plan_append(&prefix, &[d(1), d(2), d(3)]);
    let shuffled = plan_append(&prefix, &[d(3), d(1), d(2)]);
    assert_eq!(
        forward.append, shuffled.append,
        "the same append set must produce the same bytes regardless of \
                the order retrieval happened to return"
    );
}

#[test]
fn duplicates_within_one_turn_collapse() {
    let prefix = SessionPrefix::new();
    let plan = plan_append(&prefix, &[d(1), d(1), d(2), d(1)]);
    assert_eq!(plan.append, vec![d(1), d(2)]);
}

#[test]
fn replanning_after_applying_is_a_fixed_point() {
    let mut prefix = SessionPrefix::new();
    let retrieved = vec![d(1), d(2), d(3)];
    prefix.apply(&plan_append(&prefix.clone(), &retrieved));

    let again = plan_append(&prefix, &retrieved);
    assert!(
        again.append.is_empty(),
        "nothing new to say the second time"
    );
    assert_eq!(again.suppressed.len(), 3);
}

#[test]
fn supersession_appends_and_leaves_the_retired_record_in_place() {
    let mut prefix = SessionPrefix::new();
    let retired = d(1);
    prefix.apply(&plan_append(&prefix.clone(), &[retired.clone(), d(2)]));

    let replacement = d(7);
    let plan = plan_supersession(&prefix, &replacement);
    prefix.apply(&plan);

    assert!(
        prefix.contains(&retired),
        "the retired record STAYS: removing it would invalidate every token \
             after it and erase the evidence that the belief changed"
    );
    assert_eq!(
        prefix.emitted.last().unwrap(),
        &replacement,
        "the correction is appended, so the reader sees it in the order learned"
    );
    assert_eq!(prefix.emitted, vec![d(1), d(2), d(7)]);
}

// ---------------------------------------------------------------------------
// H1' — cost
// ---------------------------------------------------------------------------

/// A session where turns share most of their retrieved records, which is the
/// realistic shape: successive questions about one investigation keep pulling
/// the same core evidence.
fn overlapping_session(turns: usize) -> Vec<Vec<String>> {
    let core: Vec<String> = (1..=6).map(d).collect();
    (0..turns)
        .map(|i| {
            let mut t = core.clone();
            t.push(d(100 + i as u8)); // one new record per turn
            t
        })
        .collect()
}

#[test]
fn append_only_costs_less_than_rebuilding_and_the_gap_widens() {
    let cost = |_: &str| 200u32; // every record 200 tokens

    let mut gaps = Vec::new();
    for n in [2usize, 5, 10] {
        let turns = overlapping_session(n);
        let a = simulate_append_only(&turns, &cost, CFG);
        let r = simulate_rebuild_per_turn(&turns, &cost, CFG);

        assert!(
            a.total_prefill_tokens < r.total_prefill_tokens,
            "n={n}: append-only {} should beat rebuild {}",
            a.total_prefill_tokens,
            r.total_prefill_tokens
        );
        gaps.push(r.total_prefill_tokens - a.total_prefill_tokens);
    }

    assert!(
        gaps[0] < gaps[1] && gaps[1] < gaps[2],
        "the gap must WIDEN with session length, not merely exist: {gaps:?}"
    );
}

#[test]
fn the_metric_is_total_prefill_not_peak_context() {
    // A rebuild-per-turn assembler wins on peak context and loses badly on the
    // thing that actually costs time. Optimising the peak is how a design looks
    // successful while being slower.
    let cost = |_: &str| 200u32;
    let turns = overlapping_session(10);
    let a = simulate_append_only(&turns, &cost, CFG);
    let r = simulate_rebuild_per_turn(&turns, &cost, CFG);

    assert!(
        r.peak_context_tokens < a.peak_context_tokens,
        "rebuild does win on peak context ({} < {})",
        r.peak_context_tokens,
        a.peak_context_tokens
    );
    assert!(
        r.total_prefill_tokens > a.total_prefill_tokens,
        "...and loses on total prefill ({} > {}), which is the one that costs seconds",
        r.total_prefill_tokens,
        a.total_prefill_tokens
    );
}

#[test]
fn eviction_is_a_counted_cliff_not_a_silent_cost() {
    let cost = |_: &str| 1000u32;
    let tight = AssemblerConfig {
        context_budget_tokens: 3500,
    };
    // Each turn brings one new 1000-token record; the budget holds 3.
    let turns: Vec<Vec<String>> = (0..8).map(|i| vec![d(i as u8)]).collect();

    let a = simulate_append_only(&turns, &cost, tight);
    assert!(a.prefix_invalidations > 0, "the cliff must be reached");
    assert!(
        a.peak_context_tokens <= tight.context_budget_tokens,
        "and the budget must actually bound the context"
    );

    // Polarity: with a budget that fits, there is no cliff at all.
    let roomy = AssemblerConfig {
        context_budget_tokens: 100_000,
    };
    assert_eq!(
        simulate_append_only(&turns, &cost, roomy).prefix_invalidations,
        0
    );
}

#[test]
fn a_session_with_no_overlap_gains_nothing_and_that_is_reported_honestly() {
    // If every turn retrieves entirely fresh records, append-only has no reuse to
    // exploit. It must not appear to win by accounting sleight-of-hand.
    let cost = |_: &str| 100u32;
    let turns: Vec<Vec<String>> = (0..6).map(|i| vec![d(i as u8), d(50 + i as u8)]).collect();
    let a = simulate_append_only(&turns, &cost, CFG);
    let r = simulate_rebuild_per_turn(&turns, &cost, CFG);
    assert_eq!(
        a.total_prefill_tokens, r.total_prefill_tokens,
        "no overlap means no saving, and the numbers should say so"
    );
}
