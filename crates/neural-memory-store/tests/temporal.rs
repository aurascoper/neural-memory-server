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
