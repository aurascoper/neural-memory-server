//! Two time axes. See `docs/temporal-queries.md` for the three queries that
//! justify these existing; it was written before the migration.

use neural_memory_domain::*;
use neural_memory_store::*;

fn write(s: &Store, text: &str, occurred: &str, recorded: &str) -> String {
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: text.into(),
            evidence_class: EvidenceClass::ExternalClaim,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![],
            harness_run_id: None,
        },
        occurred_at: Some(occurred),
        recorded_at: Some(recorded),
        derivation: None,
    };
    s.put_memory(WriteChannel::Operator, &w).unwrap().0
}

/// The real shape from this session: a claim believed, then retired.
fn seeded() -> (Store, String, String) {
    let s = Store::open_in_memory().unwrap();
    let old = write(
        &s,
        "Vulkan is blocked because the policy has no backend dimension",
        "2026-07-29T00:00:00Z",
        "2026-07-29T00:00:00Z",
    );
    let new = write(
        &s,
        "Vulkan is blocked by the measurement corpus, not the schema",
        "2026-07-30T00:00:00Z",
        "2026-07-30T00:00:00Z",
    );
    s.supersede(&old, &new, "2026-07-30T00:00:00Z").unwrap();
    (s, old, new)
}

// ---------------------------------------------------------------------------
// Q1 -- was a past decision reasonable on the evidence then available?
// ---------------------------------------------------------------------------

#[test]
fn belief_can_be_reconstructed_at_a_past_point() {
    let (s, old, new) = seeded();
    let before = s.seq_at("2026-07-29T12:00:00Z").unwrap();
    let SeqAt::Resolved { seq: before } = before else {
        panic!("expected a resolved sequence, got {before:?}")
    };

    // As of the 29th the retired claim was the current belief, and its
    // replacement did not exist. That is the whole point: auditing a decision
    // needs the belief as it stood, not as later corrected.
    assert_eq!(s.belief_at(&old, before).unwrap(), Some(BeliefAt::Current));
    assert_eq!(
        s.belief_at(&new, before).unwrap(),
        Some(BeliefAt::NotYetKnown)
    );

    // Today the positions are reversed.
    let now = s.max_recorded_seq().unwrap();
    assert_eq!(s.belief_at(&old, now).unwrap(), Some(BeliefAt::Retired));
    assert_eq!(s.belief_at(&new, now).unwrap(), Some(BeliefAt::Current));
}

#[test]
fn current_as_of_returns_the_belief_of_the_day() {
    let (s, old, new) = seeded();
    let SeqAt::Resolved { seq } = s.seq_at("2026-07-29T12:00:00Z").unwrap() else {
        panic!()
    };
    let then: Vec<String> = s
        .current_as_of(seq, 50)
        .unwrap()
        .into_iter()
        .map(|e| e.record_digest)
        .collect();
    assert!(then.contains(&old), "the retired claim was current then");
    assert!(!then.contains(&new), "its replacement did not exist yet");

    // Polarity: today it is the other way round.
    let now: Vec<String> = s
        .current_as_of(s.max_recorded_seq().unwrap(), 50)
        .unwrap()
        .into_iter()
        .map(|e| e.record_digest)
        .collect();
    assert!(now.contains(&new) && !now.contains(&old));
}

#[test]
fn a_correction_recorded_later_does_not_rewrite_the_past() {
    // The append-only property, stated temporally: retiring a claim today must
    // not make it retired yesterday.
    let (s, old, _) = seeded();
    let SeqAt::Resolved { seq } = s.seq_at("2026-07-29T12:00:00Z").unwrap() else {
        panic!()
    };
    assert_eq!(s.belief_at(&old, seq).unwrap(), Some(BeliefAt::Current));
}

// ---------------------------------------------------------------------------
// Q2 -- what was known, about a window, by a point in time
// ---------------------------------------------------------------------------

#[test]
fn valid_time_and_transaction_time_are_independent() {
    // A claim ABOUT last year, recorded today. Neither axis alone finds it: the
    // valid-time window says nothing about what was known, and the sequence
    // says nothing about what the record was about.
    let s = Store::open_in_memory().unwrap();
    let old_news = write(
        &s,
        "the run on the 1st produced 8.38 tokens per second",
        "2026-07-01T00:00:00Z", // valid: July 1st
        "2026-07-31T00:00:00Z", // recorded: July 31st
    );
    let SeqAt::Resolved { seq: early } = s.seq_at("2026-07-31T00:00:00Z").unwrap() else {
        panic!()
    };

    // In the July window, known by now: found.
    let found = s
        .valid_in_window_known_by("2026-07-01T00:00:00Z", "2026-07-02T00:00:00Z", early, 50)
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].record_digest, old_news);

    // Same window, but known by a point BEFORE it was recorded: not found.
    let earlier = s
        .valid_in_window_known_by("2026-07-01T00:00:00Z", "2026-07-02T00:00:00Z", 0, 50)
        .unwrap();
    assert!(earlier.is_empty(), "it was not known that early");
}

// ---------------------------------------------------------------------------
// Q3 -- did a claim rest on evidence already retired when it was made?
// ---------------------------------------------------------------------------

#[test]
fn a_claim_citing_already_retired_evidence_is_detected() {
    let (s, old, new) = seeded();
    // A claim written AFTER `old` was retired, still resting on it.
    let stale = write(
        &s,
        "therefore the schema must gain a backend dimension",
        "2026-07-31T00:00:00Z",
        "2026-07-31T00:00:00Z",
    );
    s.add_edge(&stale, &old, "derivedFrom", "2026-07-31T00:00:00Z")
        .unwrap();

    let bad = s.cited_stale_evidence(&stale).unwrap();
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0].0, old);

    // Polarity: resting on the live replacement is not a defect...
    let sound = write(
        &s,
        "so the corpus is the blocker",
        "2026-07-31T00:00:00Z",
        "2026-07-31T00:00:00Z",
    );
    s.add_edge(&sound, &new, "derivedFrom", "2026-07-31T00:00:00Z")
        .unwrap();
    assert!(s.cited_stale_evidence(&sound).unwrap().is_empty());

    // ...and neither is a claim that PREDATES the retirement of what it cites.
    // That is ordinary history, not a defect, and conflating them would flag
    // every superseded chain in the store.
    let (s2, old2, new2) = seeded();
    let contemporaneous = s2
        .conn
        .query_row(
            "SELECT record_digest FROM memories WHERE record_digest = ?1",
            [&new2],
            |r| r.get::<_, String>(0),
        )
        .unwrap();
    s2.add_edge(
        &contemporaneous,
        &old2,
        "supersedes",
        "2026-07-30T00:00:00Z",
    )
    .unwrap();
    assert!(
        s2.cited_stale_evidence(&contemporaneous)
            .unwrap()
            .is_empty(),
        "the replacement cites what it replaces; that is not stale"
    );
}

// ---------------------------------------------------------------------------
// The honest gap
// ---------------------------------------------------------------------------

#[test]
fn an_untimestamped_record_reports_unknown_rather_than_guessing() {
    // Records written before migration 0003 have a transaction-time ORDERING
    // but no instant. Inventing one would fabricate the precision the two-axis
    // split exists to avoid.
    let s = Store::open_in_memory().unwrap();
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: "a record with no recorded_at".into(),
            evidence_class: EvidenceClass::ExternalClaim,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![],
            harness_run_id: None,
        },
        occurred_at: Some("2026-07-01T00:00:00Z"),
        recorded_at: None,
        derivation: None,
    };
    s.put_memory(WriteChannel::Operator, &w).unwrap();

    match s.seq_at("2026-07-30T00:00:00Z").unwrap() {
        SeqAt::UnknownBefore { earliest_known_seq } => {
            assert_eq!(earliest_known_seq, Some(1));
        }
        other => panic!("expected UnknownBefore, got {other:?}"),
    }
}

#[test]
fn an_empty_store_says_so_rather_than_reporting_unknown() {
    // Distinguishing "nothing that early" from "unknown that early" is the
    // point of the enum; collapsing them would make an empty store look like a
    // data-quality problem.
    let s = Store::open_in_memory().unwrap();
    assert_eq!(
        s.seq_at("2026-07-30T00:00:00Z").unwrap(),
        SeqAt::BeforeAnyRecord
    );
}

#[test]
fn a_retirement_cannot_predate_the_record_it_retires() {
    let (s, old, _) = seeded();
    let err = s.conn.execute(
        "UPDATE memories SET superseded_seq = 0 WHERE record_digest = ?1",
        [&old],
    );
    assert!(
        err.is_err(),
        "the trigger must reject an impossible ordering"
    );
}

#[test]
fn belief_reconstruction_tolerates_gaps_in_the_sequence() {
    // The live store has gaps: SQLite's AUTOINCREMENT burns a value on any
    // statement that fails inside a transaction, so recorded_seq is dense only
    // by luck. as_of compares <= against it and must never assume density.
    //
    // What WOULD break belief reconstruction is reuse or non-monotonicity.
    // Those are the properties asserted here; the gap is not a defect.
    let s = Store::open_in_memory().unwrap();
    let a = write(&s, "first", "2026-07-01T00:00:00Z", "2026-07-01T00:00:00Z");

    // Burn some sequence values the way a failed statement would.
    // sqlite_sequence has no UNIQUE on `name`, so ON CONFLICT does not apply.
    s.conn
        .execute(
            "UPDATE sqlite_sequence SET seq = 50 WHERE name = 'memories'",
            [],
        )
        .unwrap();
    let b = write(&s, "second", "2026-07-02T00:00:00Z", "2026-07-02T00:00:00Z");

    let seqs: Vec<i64> = {
        let mut st = s
            .conn
            .prepare("SELECT recorded_seq FROM memories ORDER BY recorded_seq")
            .unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .map(|x| x.unwrap())
            .collect()
    };
    assert!(
        seqs[1] - seqs[0] > 1,
        "the fixture must actually leave a gap"
    );
    assert!(seqs.windows(2).all(|w| w[1] > w[0]), "monotonic");

    // Reconstruction still works across the gap in both directions.
    let SeqAt::Resolved { seq } = s.seq_at("2026-07-01T12:00:00Z").unwrap() else {
        panic!()
    };
    assert_eq!(s.belief_at(&a, seq).unwrap(), Some(BeliefAt::Current));
    assert_eq!(s.belief_at(&b, seq).unwrap(), Some(BeliefAt::NotYetKnown));

    // A point INSIDE the gap is a legitimate query and must behave sanely.
    let mid = (seqs[0] + seqs[1]) / 2;
    assert_eq!(s.belief_at(&a, mid).unwrap(), Some(BeliefAt::Current));
    assert_eq!(s.belief_at(&b, mid).unwrap(), Some(BeliefAt::NotYetKnown));
    assert_eq!(s.current_as_of(mid, 10).unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// Standing -- which rules were IN FORCE at a point in transaction time
//
// This is the question the whole mechanism exists to answer, and the tests use
// the real dates, because the real answer is the uncomfortable one.
// ---------------------------------------------------------------------------

fn decision(s: &Store, claim: &str, at: &str) -> String {
    let w = MemoryWrite {
        terms: MemoryRecordTerms {
            claim: claim.into(),
            evidence_class: EvidenceClass::HumanDecision,
            source_artifact_sha256: None,
            source_locator: None,
            observation_identities: vec![],
            harness_run_id: None,
        },
        occurred_at: Some(at),
        recorded_at: Some(at),
        derivation: None,
    };
    s.put_memory(WriteChannel::Operator, &w).unwrap().0
}

/// The worktree rule as it actually happened: the failure recurred on the 28th,
/// the rule was declared on the 29th.
fn worktree_rule() -> (Store, String) {
    let s = Store::open_in_memory().unwrap();
    // A record predating the rule, so "the 28th" resolves to a real sequence
    // rather than falling off the front of the store.
    decision(
        &s,
        "the coupled path is gated by RADIODIALYSIS: BLOCKED",
        "2026-08-27T00:00:00Z",
    );
    let d = decision(
        &s,
        "agent sessions get their own git worktree, never a shared checkout",
        "2026-08-29T00:00:00Z",
    );
    s.declare_standing(WriteChannel::Operator, &d, "2026-08-29T00:00:00Z")
        .unwrap();
    (s, d)
}

fn seq_of(s: &Store, instant: &str) -> i64 {
    match s.seq_at(instant).unwrap() {
        SeqAt::Resolved { seq } => seq,
        other => panic!("{instant} did not resolve: {other:?}"),
    }
}

#[test]
fn the_worktree_rule_was_not_in_force_on_the_twenty_eighth() {
    // The headline. On 2026-08-28 two commits landed on the wrong branch, and
    // nobody could say whether the discipline that prevents it was binding and
    // ignored, or simply not binding -- because it lived in a plan file, and a
    // plan file has no notion of in force.
    //
    // The answer is that it was NOT in force. That is the honest one, and being
    // able to state it rather than reconstruct it from a file's mtime is the
    // whole point.
    let (s, _d) = worktree_rule();
    let seq = seq_of(&s, "2026-08-28T00:00:00Z");
    assert!(
        s.standing_as_of(seq).unwrap().is_empty(),
        "the rule was declared on the 29th and cannot have bound on the 28th"
    );
}

#[test]
fn the_worktree_rule_is_in_force_from_the_twenty_ninth() {
    // Polarity. Without it, a standing_as_of that returned nothing ever would
    // satisfy the test above and answer every historical question with silence.
    let (s, d) = worktree_rule();
    let held = s.standing_as_of(seq_of(&s, "2026-08-29T00:00:00Z")).unwrap();
    assert_eq!(held.len(), 1, "the rule must be in force on its own date");
    assert_eq!(held[0].record_digest, d);
    assert!(held[0].claim.contains("never a shared checkout"));
}

#[test]
fn retiring_a_rule_ends_it_without_erasing_that_it_bound() {
    // One property, two halves: retirement must end the rule going forward AND
    // leave the past intact. A mechanism that forgot a rule had ever bound could
    // not audit the sessions it governed.
    let (s, d) = worktree_rule();
    let while_in_force = seq_of(&s, "2026-08-29T00:00:00Z");
    assert_eq!(s.standing_as_of(while_in_force).unwrap().len(), 1);

    // Time passes and the store grows, so the retraction lands at a later
    // watermark than the declaration -- otherwise it would read as retroactive.
    decision(&s, "an unrelated later decision", "2026-08-30T00:00:00Z");
    s.retract(&d, "replaced by a stricter rule", "2026-08-31T00:00:00Z")
        .unwrap();

    let now: i64 = s
        .conn
        .query_row("SELECT max(recorded_seq) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert!(
        s.standing_as_of(now).unwrap().is_empty(),
        "a retracted rule must stop binding"
    );
    assert_eq!(
        s.standing_as_of(while_in_force).unwrap().len(),
        1,
        "and the window in which it did bind must remain auditable"
    );
}
