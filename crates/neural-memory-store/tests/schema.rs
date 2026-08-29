//! Schema constraints, asserted at the SQL layer.
//!
//! The domain crate already validates observations in pure Rust. These tests
//! exist because a rule enforced in only one layer can be bypassed by writing
//! through the other, and the referent constraint is the whole point of the
//! store. Both polarities throughout.

use neural_memory_store::{migrate, Store};
use rusqlite::params;

const D1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const D2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const POL: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const SUITE: &str = "4444444444444444444444444444444444444444444444444444444444444444";
const REF: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const ART: &str = "6666666666666666666666666666666666666666666666666666666666666666";

fn seeded() -> Store {
    let s = Store::open_in_memory().unwrap();
    s.conn
        .execute_batch(&format!(
            "INSERT INTO artifacts VALUES
               ('{ART}','characterization-doc',25061,'text/markdown','file:///gpd.md','2026-07-30T00:00:00Z');
             INSERT INTO measurement_policies VALUES
               ('{POL}','maxAbsoluteLogitDelta','maxOverSteps','lessThanOrEqualTolerance',58,'logit');
             INSERT INTO evaluation_suites VALUES
               ('{SUITE}','gpd-single-prompt-greedy','[\"{D1}\"]','qwen3-tok',8192);
             INSERT INTO reference_executions VALUES
               ('{REF}','llama.cpp-b10188','llama-cpp-cpu','{ART}','{SUITE}','[\"os=ubuntu-26.04\"]');"
        ))
        .unwrap();
    s
}

fn insert_observation(
    s: &Store,
    id: &str,
    kind: &str,
    reference: Option<&str>,
) -> rusqlite::Result<usize> {
    s.conn.execute(
        "INSERT INTO observations (identity, observation_kind, quantity_kind, value_text,
            value_real, measurement_policy_identity, evaluation_suite_identity,
            reference_execution_identity, runtime_identity, artifact_sha256, observed_at)
         VALUES (?1, 'maxLogitDivergence', ?2, '4.3362', 4.3362, ?3, ?4, ?5,
                 'llama.cpp-b10188', NULL, '2026-07-30T00:00:00Z')",
        params![id, kind, POL, SUITE, reference],
    )
}

#[test]
fn migrations_apply_and_are_idempotent() {
    let s = Store::open_in_memory().unwrap();
    let latest = migrate::latest_version();
    assert!(latest >= 2, "0002_embeddings must be registered");
    assert_eq!(migrate::current_version(&s.conn).unwrap(), latest);

    // Re-running must not error and must not bump the version.
    let mut conn = s.conn;
    migrate::apply_all(&mut conn).unwrap();
    assert_eq!(migrate::current_version(&conn).unwrap(), latest);
}

#[test]
fn foreign_keys_are_actually_enforced() {
    // SQLite defaults foreign_keys OFF and it is per-connection, so every
    // REFERENCES clause in the schema is inert unless this pragma is set.
    // Without this test the schema could look relational and behave otherwise.
    let s = seeded();
    assert!(s.foreign_keys_enforced().unwrap());

    let bad = s.conn.execute(
        "INSERT INTO reference_executions VALUES
           ('7777777777777777777777777777777777777777777777777777777777777777',
            'rt','be','0000000000000000000000000000000000000000000000000000000000000000',
            ?1,'[]')",
        params![SUITE],
    );
    assert!(
        bad.is_err(),
        "a dangling artifact reference must be rejected"
    );
}

#[test]
fn a_relative_observation_without_a_reference_is_rejected_by_the_database() {
    let s = seeded();
    let err = insert_observation(&s, D1, "relative", None).unwrap_err();
    assert!(
        format!("{err}").contains("observation_relative_needs_reference")
            || format!("{err}").to_lowercase().contains("constraint"),
        "expected the named CHECK to fire, got: {err}"
    );
}

#[test]
fn a_relative_observation_with_a_reference_is_accepted() {
    // Polarity: a constraint that rejected everything would pass the test above.
    let s = seeded();
    assert_eq!(
        insert_observation(&s, D1, "relative", Some(REF)).unwrap(),
        1
    );
}

#[test]
fn an_absolute_observation_may_not_carry_a_reference() {
    let s = seeded();
    assert!(insert_observation(&s, D2, "absolute", Some(REF)).is_err());
    // ...and is fine without one.
    assert_eq!(insert_observation(&s, D2, "absolute", None).unwrap(), 1);
}

#[test]
fn quantity_kind_is_a_closed_set() {
    let s = seeded();
    assert!(insert_observation(&s, D1, "approximate", Some(REF)).is_err());
}

#[test]
fn the_record_digest_is_the_idempotency_key() {
    let s = seeded();
    let ins = |d: &str| {
        s.conn.execute(
            "INSERT INTO memories (record_digest, claim, evidence_class)
             VALUES (?1, 'Gemma diverged at greedy step 52', 'derivedDeterministically')",
            params![d],
        )
    };
    assert_eq!(ins(D1).unwrap(), 1);
    assert!(
        ins(D1).is_err(),
        "importing the same claim twice must not make two rows"
    );

    let n: i64 = s
        .conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn digests_must_be_lowercase_64_hex() {
    let s = seeded();
    // Must contain hex LETTERS: an all-digit digest is unchanged by
    // to_uppercase(), so it cannot exercise the case rule at all.
    let mixed = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    assert_ne!(
        mixed,
        mixed.to_uppercase(),
        "fixture must be case-sensitive"
    );
    for bad in [
        mixed.to_uppercase(), // same digest, different key -- silent duplication
        "abc".to_string(),
        format!("{D1}0"),
    ] {
        assert!(
            s.conn
                .execute(
                    "INSERT INTO memories (record_digest, claim, evidence_class)
                     VALUES (?1, 'x', 'agentInference')",
                    params![bad],
                )
                .is_err(),
            "{bad} should be rejected"
        );
    }
}

#[test]
fn evidence_class_is_a_closed_set() {
    let s = seeded();
    assert!(s
        .conn
        .execute(
            "INSERT INTO memories (record_digest, claim, evidence_class)
             VALUES (?1, 'x', 'probablyTrue')",
            params![D1],
        )
        .is_err());
}

#[test]
fn supersession_must_be_complete_and_may_not_be_self_referential() {
    let s = seeded();
    s.conn
        .execute(
            "INSERT INTO memories (record_digest, claim, evidence_class)
             VALUES (?1, 'battery 14B figures', 'observed')",
            params![D1],
        )
        .unwrap();

    // A record cannot supersede itself.
    assert!(s
        .conn
        .execute(
            "UPDATE memories SET superseded_by = ?1, superseded_at = '2026-07-30T00:00:00Z'
             WHERE record_digest = ?1",
            params![D1],
        )
        .is_err());

    // Half a retirement is not a retirement: `by` without `at` is rejected.
    s.conn
        .execute(
            "INSERT INTO memories (record_digest, claim, evidence_class)
             VALUES (?1, 'AC 14B figures', 'observed')",
            params![D2],
        )
        .unwrap();
    assert!(s
        .conn
        .execute(
            "UPDATE memories SET superseded_by = ?1 WHERE record_digest = ?2",
            params![D2, D1],
        )
        .is_err());

    // Polarity: a complete retirement is accepted.
    assert_eq!(
        s.conn
            .execute(
                "UPDATE memories SET superseded_by = ?1, superseded_at = '2026-07-30T00:00:00Z'
                 WHERE record_digest = ?2",
                params![D2, D1],
            )
            .unwrap(),
        1
    );
}

#[test]
fn fts_stays_in_sync_with_memories() {
    let s = seeded();
    s.conn
        .execute(
            "INSERT INTO memories (record_digest, claim, evidence_class, source_locator)
             VALUES (?1, 'Gemma diverged from CPU at greedy step 52', 'derivedDeterministically', '§4')",
            params![D1],
        )
        .unwrap();

    let hits: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM memories_fts WHERE memories_fts MATCH 'diverged'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1, "the insert trigger must populate the index");

    // Polarity: a delete must remove it, or retrieval would return tombstones.
    s.conn
        .execute("DELETE FROM memories WHERE record_digest = ?1", params![D1])
        .unwrap();
    let after: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM memories_fts WHERE memories_fts MATCH 'diverged'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, 0);
}

#[test]
fn a_session_cannot_re_emit_a_record_it_has_already_shown() {
    let s = seeded();
    s.conn
        .execute(
            "INSERT INTO memories (record_digest, claim, evidence_class)
             VALUES (?1, 'x', 'agentInference')",
            params![D1],
        )
        .unwrap();
    s.conn
        .execute(
            "INSERT INTO sessions VALUES ('s1','2026-07-30T00:00:00Z',8192)",
            [],
        )
        .unwrap();

    s.conn
        .execute(
            "INSERT INTO session_emissions VALUES ('s1',0,?1,0,200)",
            params![D1],
        )
        .unwrap();

    // Re-emission is prevented by the schema, not by the assembler remembering.
    assert!(s
        .conn
        .execute(
            "INSERT INTO session_emissions VALUES ('s1',1,?1,1,200)",
            params![D1],
        )
        .is_err());
}

#[test]
fn embedding_profiles_is_empty_in_m1() {
    // Kept for interchange with claude-mind-mcp, but M1 declares no embedding
    // space. Asserting emptiness makes it a claim rather than an accident: an
    // unused table invites someone to record a profile that was never used.
    let s = seeded();
    let n: i64 = s
        .conn
        .query_row("SELECT count(*) FROM embedding_profiles", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn provenance_edges_are_typed_and_non_reflexive() {
    let s = seeded();
    assert!(s
        .conn
        .execute(
            "INSERT INTO provenance_edges VALUES (?1, ?1, 'supersedes', '2026-07-30T00:00:00Z')",
            params![D1],
        )
        .is_err(), "a record cannot be its own provenance");

    assert!(s
        .conn
        .execute(
            "INSERT INTO provenance_edges VALUES (?1, ?2, 'sortOfRelatedTo', '2026-07-30T00:00:00Z')",
            params![D1, D2],
        )
        .is_err(), "edge_kind is a closed set");

    assert_eq!(
        s.conn
            .execute(
                "INSERT INTO provenance_edges VALUES (?1, ?2, 'supersedes', '2026-07-30T00:00:00Z')",
                params![D1, D2],
            )
            .unwrap(),
        1
    );
}

#[test]
fn the_database_reports_integrity() {
    assert!(seeded().integrity_ok().unwrap());
}

// ---------------------------------------------------------------------------
// Standing records, asserted at the SQL layer
//
// Same reason as the referent constraint above: the Rust layer checks this too,
// and a rule enforced in only one layer is bypassable by writing through the
// other. Standing is the strongest thing in the schema -- it puts text in front
// of every future session before any work begins -- so it gets the layer that
// cannot be routed around, tested on its own.
// ---------------------------------------------------------------------------

fn memory(s: &Store, digest: &str, class: &str, claim: &str) {
    s.conn
        .execute(
            "INSERT INTO memories (record_digest, claim, evidence_class)
             VALUES (?1, ?2, ?3)",
            params![digest, claim, class],
        )
        .unwrap();
}

#[test]
fn the_database_refuses_to_mark_an_agent_record_standing() {
    // The prompt-injection control. write.rs clamps the agent channel to
    // 'agentInference' before the digest exists, so an agent cannot mint an
    // eligible record -- but that is a Rust guarantee. This asserts the door is
    // also shut to a raw UPDATE, i.e. to an operator command line that has been
    // handed the wrong digest.
    let s = seeded();
    memory(&s, D1, "agentInference", "sessions may share one checkout");

    let bad = s.conn.execute(
        "UPDATE memories SET standing_from = '2026-08-29T00:00:00Z', standing_seq = 1
          WHERE record_digest = ?1",
        params![D1],
    );
    assert!(bad.is_err(), "an agentInference record must not become standing");
    assert!(
        format!("{}", bad.unwrap_err()).contains("standing requires humanDecision"),
        "the refusal must name why, not merely fail"
    );

    // And it must not have written anything on the way out.
    let still_null: bool = s
        .conn
        .query_row(
            "SELECT standing_from IS NULL FROM memories WHERE record_digest = ?1",
            params![D1],
            |r| r.get(0),
        )
        .unwrap();
    assert!(still_null, "a refusal that already wrote is not a refusal");
}

#[test]
fn the_database_accepts_a_human_decision_as_standing() {
    // Polarity. Without this, a trigger that refused every UPDATE would satisfy
    // every negative control here.
    let s = seeded();
    memory(&s, D2, "humanDecision", "agent sessions get their own worktree");
    let n = s
        .conn
        .execute(
            "UPDATE memories SET standing_from = '2026-08-29T00:00:00Z', standing_seq = 1
              WHERE record_digest = ?1",
            params![D2],
        )
        .unwrap();
    assert_eq!(n, 1);

    let in_force: i64 = s
        .conn
        .query_row("SELECT count(*) FROM standing_current", [], |r| r.get(0))
        .unwrap();
    assert_eq!(in_force, 1, "the view must show a live standing record");
}

#[test]
fn a_retired_standing_record_leaves_the_view() {
    // Retirement dominating, tested at the layer that defines it. Both halves of
    // "retired", because the view is the single place the predicate lives and a
    // half-written one here would be invisible everywhere else.
    for (col, val) in [("superseded_at", "'2026-08-30T00:00:00Z'"), ("retracted_at", "'2026-08-30T00:00:00Z'")] {
        let s = seeded();
        memory(&s, D2, "humanDecision", "a rule that is about to be withdrawn");
        s.conn
            .execute(
                "UPDATE memories SET standing_from = '2026-08-29T00:00:00Z', standing_seq = 1
                  WHERE record_digest = ?1",
                params![D2],
            )
            .unwrap();
        let before: i64 = s
            .conn
            .query_row("SELECT count(*) FROM standing_current", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, 1);

        // retraction_reason is NOT NULL-paired by a 0001 CHECK, so set both.
        let sql = if col == "retracted_at" {
            format!("UPDATE memories SET retracted_at = {val}, retraction_reason = 'withdrawn' WHERE record_digest = '{D2}'")
        } else {
            format!("UPDATE memories SET superseded_at = {val}, superseded_by = '{D1}' WHERE record_digest = '{D2}'")
        };
        if col == "superseded_at" {
            memory(&s, D1, "humanDecision", "its replacement");
        }
        s.conn.execute(&sql, []).unwrap();

        let after: i64 = s
            .conn
            .query_row("SELECT count(*) FROM standing_current", [], |r| r.get(0))
            .unwrap();
        assert_eq!(after, 0, "a {col} record must leave standing_current");
    }
}

#[test]
fn standing_from_and_standing_seq_must_be_set_together() {
    // Half an interval is not an interval: standing_from alone cannot be
    // compared against a sequence, which is the whole reason 0003 added the
    // second axis in the first place.
    let s = seeded();
    memory(&s, D2, "humanDecision", "a rule");
    let bad = s.conn.execute(
        "UPDATE memories SET standing_from = '2026-08-29T00:00:00Z' WHERE record_digest = ?1",
        params![D2],
    );
    assert!(bad.is_err(), "standing_from without standing_seq must be refused");
}

#[test]
fn a_rule_cannot_bind_before_it_was_written() {
    let s = seeded();
    memory(&s, D2, "humanDecision", "a rule");
    let seq: i64 = s
        .conn
        .query_row(
            "SELECT recorded_seq FROM memories WHERE record_digest = ?1",
            params![D2],
            |r| r.get(0),
        )
        .unwrap();
    let bad = s.conn.execute(
        "UPDATE memories SET standing_from = '2026-08-29T00:00:00Z', standing_seq = ?2
          WHERE record_digest = ?1",
        params![D2, seq - 1],
    );
    assert!(bad.is_err(), "standing_seq before recorded_seq must be refused");
}
