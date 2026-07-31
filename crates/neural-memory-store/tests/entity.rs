//! The entity branch: alias resolution, attribution, and the sharing law.

use neural_memory_domain::*;
use neural_memory_store::*;

const NOW: &str = "2026-07-31T00:00:00Z";

fn gemma() -> EntityTerms {
    EntityTerms {
        canonical_name: "Gemma 4 12B Q5_K_M".into(),
        entity_type: "model".into(),
        // "correctness escalation candidate" shares NO token with the record
        // text. "the 12B model" does -- 12B appears in the canonical name -- so
        // it cannot demonstrate anything the lexical branch does not already do.
        aliases: vec![
            "the 12B model".into(),
            "gemma4-12b-q5km".into(),
            "correctness escalation candidate".into(),
        ],
    }
}
fn qwen() -> EntityTerms {
    EntityTerms {
        canonical_name: "Qwen3 8B Q6_K".into(),
        entity_type: "model".into(),
        // "responsive default" shares no token with the record text either, so
        // a query made only of aliases isolates the entity branch.
        aliases: vec!["the 8B model".into(), "responsive default".into()],
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

fn seeded() -> (Store, String, String) {
    let s = Store::open_in_memory().unwrap();
    s.put_entity(&gemma()).unwrap();
    s.put_entity(&qwen()).unwrap();
    let g = claim(
        &s,
        "Gemma 4 12B Q5_K_M exceeded the preregistered tolerance",
    );
    let q = claim(&s, "Qwen3 8B Q6_K stayed within the carve-out");
    s.reindex_mentions().unwrap();
    (s, g, q)
}

fn opts(query: &str) -> RecallOptions<'_> {
    RecallOptions {
        query,
        entities: true,
        semantic: None,
        as_of: NOW,
        limit: 20,
        max_hops: 1,
        include_retired: false,
    }
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

#[test]
fn longest_match_wins_and_spans_do_not_overlap() {
    // Reporting both "Qwen3 8B" and "Qwen3 8B Q6_K" would make the shorter a
    // hit on every mention of the longer and quietly inflate every score.
    let short = EntityTerms {
        canonical_name: "Qwen3 8B".into(),
        entity_type: "model".into(),
        aliases: vec![],
    };
    let dict = EntityDictionary::new(&[short.clone(), qwen()]);
    let ms = extract_mentions(&dict, "we benchmarked Qwen3 8B Q6_K today");
    assert_eq!(ms.len(), 1);
    assert_eq!(ms[0].surface, "Qwen3 8B Q6_K");
    assert_eq!(ms[0].entity_identity, entity_identity(&qwen()));
}

#[test]
fn matching_is_word_bounded() {
    let dict = EntityDictionary::new(&[qwen()]);
    // Substring of a longer token must not match.
    assert!(extract_mentions(&dict, "xQwen3 8B Q6_Ky").is_empty());
    // Polarity: punctuation-delimited does match.
    assert_eq!(extract_mentions(&dict, "(Qwen3 8B Q6_K)").len(), 1);
}

#[test]
fn matching_is_case_insensitive_and_spans_point_at_the_source() {
    let dict = EntityDictionary::new(&[gemma()]);
    let text = "GEMMA 4 12B Q5_K_M diverged";
    let ms = extract_mentions(&dict, text);
    assert_eq!(ms.len(), 1);
    // The span must index the ORIGINAL text, in its original casing, so it can
    // be checked without re-running the extractor.
    assert_eq!(&text[ms[0].start..ms[0].end], "GEMMA 4 12B Q5_K_M");
}

#[test]
fn the_extractor_identity_covers_the_dictionary_not_just_the_algorithm() {
    // A mention must name WHICH dictionary found it. Adding an entity changes
    // what the same text yields, so it must change the identity.
    let a = EntityDictionary::new(&[gemma()]);
    let b = EntityDictionary::new(&[gemma(), qwen()]);
    assert_ne!(a.extractor_identity(), b.extractor_identity());
    // Polarity: order of declaration is not substantive.
    let c = EntityDictionary::new(&[qwen(), gemma()]);
    assert_eq!(b.extractor_identity(), c.extractor_identity());
}

#[test]
fn an_alias_changes_the_entity_identity() {
    let mut g = gemma();
    g.aliases.push("big gemma".into());
    assert_ne!(entity_identity(&gemma()), entity_identity(&g));
    // Polarity: alias ORDER does not.
    let mut reordered = gemma();
    reordered.aliases.reverse();
    assert_eq!(entity_identity(&gemma()), entity_identity(&reordered));
}

// ---------------------------------------------------------------------------
// The branch
// ---------------------------------------------------------------------------

#[test]
fn an_alias_query_finds_the_record_that_never_uses_it() {
    // THE justification for the branch. The record says "Gemma 4 12B Q5_K_M
    // exceeded the preregistered tolerance"; the query says "correctness
    // escalation candidate". Not one token in common.
    let (s, g, _) = seeded();
    let r = s.recall(&opts("correctness escalation candidate")).unwrap();
    let hit = r
        .hits
        .iter()
        .find(|h| h.record_digest == g)
        .expect("alias must resolve to the record");
    assert!(hit.branches.contains(&Branch::Entity));
    assert_eq!(hit.shared_entities, Some(1));

    // ...and lexical alone does NOT find it, or the branch is redundant here.
    let mut lex_only = opts("correctness escalation candidate");
    lex_only.entities = false;
    let without = s.recall(&lex_only).unwrap();
    assert!(
        !without.hits.iter().any(|h| h.record_digest == g),
        "if lexical already found it, this test proves nothing about the branch"
    );
}

#[test]
fn a_record_found_by_both_lexical_and_entity_reports_both() {
    let (s, g, _) = seeded();
    let r = s.recall(&opts("Gemma 4 12B Q5_K_M tolerance")).unwrap();
    let hit = r.hits.iter().find(|h| h.record_digest == g).unwrap();
    assert!(hit.branches.contains(&Branch::Lexical));
    assert!(hit.branches.contains(&Branch::Entity));
    assert_eq!(
        r.counts.unique,
        r.hits.len(),
        "attribution must not double-count"
    );
}

#[test]
fn a_query_naming_no_entity_runs_no_entity_branch() {
    // Polarity: a branch that fired on everything would be no signal.
    let (s, _, _) = seeded();
    let r = s.recall(&opts("thermal throttling behaviour")).unwrap();
    assert_eq!(r.counts.entity, 0);
    assert!(r.hits.iter().all(|h| h.shared_entities.is_none()));
}

#[test]
fn more_shared_entities_outranks_fewer() {
    // The query is built ONLY from aliases that share no token with any record,
    // so lexical contributes nothing and the entity branch is what decides.
    // An earlier version of this test used a query with real words in it and
    // failed -- correctly: it asserted a ceteris-paribus claim while letting
    // bm25 vary, and bm25 favoured the shorter record.
    let (s, _, _) = seeded();
    let both = claim(&s, "Gemma 4 12B Q5_K_M and Qwen3 8B Q6_K were compared");
    s.reindex_mentions().unwrap();

    let q = "correctness escalation candidate responsive default";
    let r = s.recall(&opts(q)).unwrap();
    assert!(
        r.hits.iter().all(|h| h.lexical_score.is_none()),
        "the query must share no token with any record, or this tests the blend"
    );
    let top = &r.hits[0];
    assert_eq!(top.record_digest, both);
    assert_eq!(top.shared_entities, Some(2));
    assert!(r.hits.iter().any(|h| h.shared_entities == Some(1)));
}

#[test]
fn retired_records_are_excluded_from_the_entity_branch_too() {
    // Otherwise this branch quietly reintroduces what the others exclude.
    let (s, g, q) = seeded();
    s.supersede(&g, &q, NOW).unwrap();
    let r = s.recall(&opts("the 12B model")).unwrap();
    assert!(r.hits.iter().all(|h| h.record_digest != g));
    assert!(r.withheld_retired.contains(&g));
}

#[test]
fn reindexing_replaces_rather_than_accumulates() {
    // Re-running a DIFFERENT dictionary must not leave behind spans the current
    // one would never produce.
    let (s, _, _) = seeded();
    let first: i64 = s
        .conn
        .query_row("SELECT count(*) FROM mentions", [], |r| r.get(0))
        .unwrap();
    s.reindex_mentions().unwrap();
    let second: i64 = s
        .conn
        .query_row("SELECT count(*) FROM mentions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(first, second, "mentions must not accumulate on re-index");
}

#[test]
fn every_mention_records_which_dictionary_found_it() {
    // mentions.extractor_identity is NOT NULL in the first migration: a mention
    // has to say what found it, which is why this branch uses a declared
    // dictionary rather than a black-box model.
    let (s, _, _) = seeded();
    let (dict, _) = s.entity_dictionary().unwrap();
    let expected = dict.extractor_identity();
    let mut st = s
        .conn
        .prepare("SELECT DISTINCT extractor_identity FROM mentions")
        .unwrap();
    let ids: Vec<String> = st
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|x| x.unwrap())
        .collect();
    assert_eq!(ids, vec![expected]);
}

#[test]
fn a_direct_entity_match_outranks_a_bare_provenance_neighbour() {
    // Calibration, found on live data. With the obvious `1 - 1/(1+n)` curve a
    // single shared entity scored 0.20*0.5 = 0.10, exactly tying a one-hop
    // graph neighbour at 0.10*1.0 -- so an alias-only query returned the record
    // it actually named level with records merely adjacent to something.
    let (s, g, _) = seeded();
    let neighbour = claim(&s, "an unrelated note about packaging");
    s.add_edge(&g, &neighbour, "supports", NOW).unwrap();
    s.reindex_mentions().unwrap();

    let r = s.recall(&opts("correctness escalation candidate")).unwrap();
    assert!(
        r.hits.iter().all(|h| h.lexical_score.is_none()),
        "the query must share no token with any record"
    );
    let named = r.hits.iter().position(|h| h.record_digest == g).unwrap();
    let adjacent = r
        .hits
        .iter()
        .position(|h| h.record_digest == neighbour)
        .unwrap();
    assert!(
        named < adjacent,
        "the record the query NAMES must outrank the one merely next to it"
    );
}
