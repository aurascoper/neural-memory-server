//! The bounded GPD import, checked against the pre-registered H6 rubric.
//!
//! `corpus/h6-rubric.json` was written before the importer existed. These tests
//! verify the import satisfies it — not that the rubric describes whatever the
//! import happened to produce.

use neural_memory_mcp::corpus::import_gpd;
use neural_memory_store::*;
use serde_json::Value;

fn rubric() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/h6-rubric.json");
    let raw = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&raw).expect("rubric must be valid JSON")
}

fn imported() -> Store {
    let s = Store::open_in_memory().unwrap();
    import_gpd(&s).expect("import must succeed");
    s
}

fn digest_of_claim(s: &Store, claim: &str) -> Option<String> {
    s.conn
        .query_row(
            "SELECT record_digest FROM memories WHERE claim = ?1",
            [claim],
            |r| r.get(0),
        )
        .ok()
}

#[test]
fn every_pre_registered_fact_resolves_to_exactly_one_record() {
    let s = imported();
    let r = rubric();
    let facts = r["requiredFacts"].as_array().unwrap();
    assert_eq!(facts.len(), 8, "the rubric must not have been trimmed");

    for f in facts {
        let claim = f["claim"].as_str().unwrap();
        let n: i64 = s
            .conn
            .query_row(
                "SELECT count(*) FROM memories WHERE claim = ?1",
                [claim],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 1,
            "{} must resolve to exactly one record; got {n}\n  claim: {claim}",
            f["id"]
        );
    }
}

#[test]
fn the_retired_fact_is_stored_as_retired_with_a_replacement() {
    // F7 is the decoy. It must be findable -- an agent that pattern-matches the
    // old wording should reach it -- and must be marked retired, with the
    // supersedes edge that leads to the correction.
    let s = imported();
    let r = rubric();
    let f7 = r["requiredFacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "F7")
        .unwrap();
    assert_eq!(f7["retired"], true, "the rubric must mark F7 retired");

    let digest = digest_of_claim(&s, f7["claim"].as_str().unwrap()).unwrap();
    let (by, at): (Option<String>, Option<String>) = s
        .conn
        .query_row(
            "SELECT superseded_by, superseded_at FROM memories WHERE record_digest = ?1",
            [&digest],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(
        at.is_some(),
        "F7 must be retired in the store, not merely in the rubric"
    );

    let replacement = by.expect("F7 must name its replacement");
    let edge: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM provenance_edges
             WHERE src_digest = ?1 AND dst_digest = ?2 AND edge_kind = 'supersedes'",
            [&replacement, &digest],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(edge, 1, "the supersedes edge is what arm (c) deletes");
}

#[test]
fn the_retired_fact_is_absent_from_default_recall_and_reported_as_withheld() {
    let s = imported();
    let r = rubric();
    let f7_claim = r["requiredFacts"].as_array().unwrap()[6]["claim"]
        .as_str()
        .unwrap()
        .to_string();
    let f7 = digest_of_claim(&s, &f7_claim).unwrap();

    let res = s
        .recall(&RecallOptions {
            query: "backend dimension identical seal blocked",
            as_of: "2026-07-30T12:00:00Z",
            limit: 20,
            max_hops: 1,
            include_retired: false,
        })
        .unwrap();
    assert!(
        res.hits.iter().all(|h| h.record_digest != f7),
        "the retired root-cause claim must not be returned as current"
    );
    assert!(res.withheld_retired.contains(&f7));
}

#[test]
fn no_single_record_contains_the_whole_answer() {
    // "Delete the linking prose on import." If one record carried the measured
    // value, the tolerance and the conclusion together, H6 would be one lexical
    // hop and would measure retrieval rather than provenance traversal.
    let s = imported();
    let claims: Vec<String> = {
        let mut st = s.conn.prepare("SELECT claim FROM memories").unwrap();
        let rows = st.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.map(|c| c.unwrap()).collect()
    };

    // Identifying markers for the three legs the answer must join.
    let value = "4.3362";
    let tolerance = "0.5";
    let conclusion = "own numericalContractId";

    for c in &claims {
        let legs = [value, tolerance, conclusion]
            .iter()
            .filter(|m| c.contains(*m))
            .count();
        assert!(
            legs < 2,
            "a single record carries {legs} legs of the answer, so the chain \
             need not be traversed:\n  {c}"
        );
    }
}

#[test]
fn the_answer_chain_is_reachable_by_traversal() {
    // Polarity to the test above: having split the facts apart, they must still
    // be connected -- otherwise the question is unanswerable rather than hard.
    let s = imported();
    let r = rubric();
    let claim_of = |id: &str| -> String {
        r["requiredFacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == id)
            .unwrap()["claim"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let f1 = digest_of_claim(&s, &claim_of("F1")).unwrap();

    let reachable = s.traverse(&f1, Direction::Both, 4).unwrap();
    let ids: Vec<String> = reachable.into_iter().map(|(d, _)| d).collect();

    for id in ["F2", "F3", "F4"] {
        let d = digest_of_claim(&s, &claim_of(id)).unwrap();
        assert!(
            ids.contains(&d),
            "{id} must be reachable from F1 within 4 hops"
        );
    }
}

#[test]
fn re_importing_changes_nothing() {
    let s = Store::open_in_memory().unwrap();
    let first = import_gpd(&s).unwrap();
    let seq = s.max_recorded_seq().unwrap();
    let rows: i64 = s
        .conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .unwrap();

    let second = import_gpd(&s).unwrap();
    assert!(first.contains("17 claims inserted"));
    assert!(second.contains("0 claims inserted"), "got: {second}");
    assert_eq!(s.max_recorded_seq().unwrap(), seq, "no new history");
    assert_eq!(
        s.conn
            .query_row("SELECT count(*) FROM memories", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        rows
    );
}

#[test]
fn every_relative_observation_names_its_reference() {
    // The constraint the whole store exists for, checked against real data
    // rather than a fixture.
    let s = imported();
    let orphans: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM observations
             WHERE quantity_kind = 'relative' AND reference_execution_identity IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0);

    // And the divergence figures really are stored as relative, not smuggled in
    // as absolutes to dodge the constraint.
    let relative: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM observations
             WHERE observation_kind LIKE 'maxLogitDivergence%' AND quantity_kind = 'relative'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        relative, 2,
        "both divergence figures are relative quantities"
    );
}

#[test]
fn the_import_stays_within_its_declared_bound() {
    // The importer's own doc comment fixes its scope. This is the guard against
    // the failure mode flagged in review: an import with no natural stopping
    // point that quietly grows until it has transcribed the whole document.
    let s = imported();
    let n: i64 = s
        .conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert!(
        n <= 25,
        "import has grown to {n} claims; the bound is the §4 table, the §1 \
         supersession, §12 open items and the ADR/schema excerpts. Widening it \
         is a decision to take deliberately, not by accretion."
    );
}

// ---------------------------------------------------------------------------
// The contested corpus: the gate must have something to fire on
// ---------------------------------------------------------------------------

use neural_memory_mcp::corpus_contested::import_contested;

fn contested_rubric() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/contested-rubric.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn contested() -> Store {
    let s = Store::open_in_memory().unwrap();
    import_contested(&s).expect("contested import must succeed");
    s
}

#[test]
fn both_rivals_recompute_or_the_store_would_have_refused_them() {
    // They are written as DerivedDeterministically, so the store re-runs the
    // arithmetic and rejects a mismatch. Their presence IS the proof that this
    // is a real disagreement rather than one of them being a typo.
    let s = contested();
    let n: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM memories WHERE evidence_class = 'derivedDeterministically'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        n, 2,
        "both percentage claims must have survived recomputation"
    );
}

#[test]
fn neither_rival_is_retired() {
    // Retiring one would assert the store knows which baseline the document
    // meant. It does not -- that is the finding, and it is what makes this the
    // case submit_answer exists for.
    let s = contested();
    let n: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM memories WHERE superseded_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn citing_either_rival_raises_an_obligation() {
    // The property the whole corpus exists to guarantee.
    let s = contested();
    let r = contested_rubric();
    let digest_of = |id: &str| -> String {
        let c = r["requiredFacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["id"] == id)
            .unwrap()["claim"]
            .as_str()
            .unwrap();
        digest_of_claim(&s, c).unwrap_or_else(|| panic!("{id} not imported"))
    };

    for id in ["G4", "G5"] {
        let obligations = s.conflict_obligations(&[digest_of(id)]).unwrap();
        assert_eq!(
            obligations.len(),
            1,
            "{id} must raise exactly one obligation"
        );
    }

    // Polarity: the uncontested measurements raise none, so the gate is not
    // simply firing on everything.
    for id in ["G1", "G2", "G3", "G6"] {
        assert!(
            s.conflict_obligations(&[digest_of(id)]).unwrap().is_empty(),
            "{id} must be free of obligations"
        );
    }
}

#[test]
fn there_is_no_uncontested_route_to_a_percentage() {
    // The escape H6 revealed was that a shallow answer is a conflict-free
    // answer. Here every record stating a percentage is contested, so an answer
    // that gives one cannot avoid the gate.
    let s = contested();
    let mut st = s
        .conn
        .prepare("SELECT record_digest, claim FROM memories WHERE claim LIKE '%percent%'")
        .unwrap();
    let rows: Vec<(String, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|x| x.unwrap())
        .collect();
    assert!(rows.len() >= 2);
    for (digest, claim) in rows {
        // G6 reports that the document states a figure without a baseline; it
        // is about the ambiguity rather than an assertion of a value.
        if claim.contains("without naming") {
            continue;
        }
        assert!(
            !s.conflict_obligations(&[digest]).unwrap().is_empty(),
            "a percentage claim with no obligation would be an escape route: {claim}"
        );
    }
}

#[test]
fn every_pre_registered_contested_fact_resolves() {
    let s = contested();
    let r = contested_rubric();
    for f in r["requiredFacts"].as_array().unwrap() {
        let claim = f["claim"].as_str().unwrap();
        let n: i64 = s
            .conn
            .query_row(
                "SELECT count(*) FROM memories WHERE claim = ?1",
                [claim],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "{} unresolved: {claim}", f["id"]);
    }
}
