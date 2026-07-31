//! Declarative ingestion: equivalence with the hand-written importer, and the
//! validation rules that keep the doctrine intact in a format anyone can edit.

use neural_memory_store::{ingest::ingest, ingest::validate, Store};

fn doc(extra: &str) -> String {
    format!(
        "version = 1\nrecorded_at = \"2026-07-31T00:00:00Z\"\n\n\
         [[artifact]]\nid = \"a\"\nkind = \"doc\"\n\
         sha256 = \"{}\"\nbytes = 1\nmedia_type = \"text/plain\"\nuri = \"file:///a\"\n\n\
         [[suite]]\nid = \"s\"\nname = \"s\"\ncase_texts = [\"c\"]\n\
         tokenizer = \"t\"\ncontext_cap = 8192\n\n\
         [[policy]]\nid = \"p\"\nmetric = \"m\"\naggregation = \"agg\"\n\
         comparison_rule = \"r\"\nunit = \"u\"\n\n{extra}",
        "aa".repeat(32)
    )
}

// Equivalence with the hand-written Rust corpus lives in the mcp crate's
// tests/ingest_equivalence.rs, because it needs BOTH importers and the store
// crate cannot depend on mcp.

#[test]
fn re_ingesting_the_same_document_writes_nothing() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/contested.toml"
    ))
    .unwrap();
    let s = Store::open_in_memory().unwrap();
    let first = ingest(&s, &text, false).unwrap();
    let seq = s.max_recorded_seq().unwrap();
    let second = ingest(&s, &text, false).unwrap();
    assert!(first.inserted > 0);
    assert_eq!(second.inserted, 0, "a re-run is not new history");
    assert_eq!(s.max_recorded_seq().unwrap(), seq);
}

// ---------------------------------------------------------------------------
// The doctrine survives being in a hand-editable format
// ---------------------------------------------------------------------------

#[test]
fn a_relative_observation_without_a_reference_is_rejected_at_parse_time() {
    // Before any database sees it. The message is the store's own.
    let e = validate(&doc(
        "[[observation]]\nid = \"o\"\nkind = \"k\"\nquantity = \"relative\"\n\
         value = \"1.0\"\npolicy = \"p\"\nsuite = \"s\"\nruntime = \"rt\"\n",
    ))
    .unwrap_err();
    assert!(e.contains("nothing to diverge from"), "{e}");
}

#[test]
fn an_absolute_observation_naming_a_reference_is_rejected() {
    // Scope cuts both ways: an out-of-scope field is visible, not ignored.
    let e = validate(&doc(
        "[[reference]]\nid = \"r\"\nruntime = \"rt\"\nbackend = \"b\"\n\
         artifact = \"a\"\nsuite = \"s\"\n\n\
         [[observation]]\nid = \"o\"\nkind = \"k\"\nquantity = \"absolute\"\n\
         value = \"1.0\"\npolicy = \"p\"\nsuite = \"s\"\nruntime = \"rt\"\nreference = \"r\"\n",
    ))
    .unwrap_err();
    assert!(e.contains("out of scope"), "{e}");
}

#[test]
fn a_bare_number_is_rejected_with_the_reason() {
    // The float caveat, enforced where an author would trip over it.
    let e = validate(&doc(
        "[[observation]]\nid = \"o\"\nkind = \"k\"\nquantity = \"absolute\"\n\
         value = 8.38\npolicy = \"p\"\nsuite = \"s\"\nruntime = \"rt\"\n",
    ))
    .unwrap_err();
    assert!(e.contains("QUOTED"), "{e}");
    assert!(e.contains("formatter"), "must say WHY, not just what: {e}");

    // Polarity: quoted is fine.
    assert!(validate(&doc(
        "[[observation]]\nid = \"o\"\nkind = \"k\"\nquantity = \"absolute\"\n\
         value = \"8.38\"\npolicy = \"p\"\nsuite = \"s\"\nruntime = \"rt\"\n",
    ))
    .is_ok());
}

#[test]
fn declaring_observed_does_not_make_it_so() {
    // Evidence class is still derived, not accepted, in the authoring format.
    let e = validate(&doc(
        "[[claim]]\nid = \"c\"\ntext = \"t\"\nevidence = \"observed\"\n",
    ))
    .unwrap_err();
    assert!(e.contains("not an observation of it"), "{e}");
}

#[test]
fn a_derived_claim_without_a_transform_is_rejected() {
    let e = validate(&doc(
        "[[claim]]\nid = \"c\"\ntext = \"t\"\nevidence = \"derived\"\n",
    ))
    .unwrap_err();
    assert!(e.contains("nothing can recompute it"), "{e}");
}

#[test]
fn a_derivation_that_does_not_recompute_is_still_refused_on_ingest() {
    // Parse-time validation cannot check arithmetic; the store does, and the
    // ingest path does not get a way around it.
    let s = Store::open_in_memory().unwrap();
    let text = doc(
        "[[observation]]\nid = \"x\"\nkind = \"k\"\nquantity = \"absolute\"\n\
         value = \"10.0\"\npolicy = \"p\"\nsuite = \"s\"\nruntime = \"rt\"\n\n\
         [[observation]]\nid = \"y\"\nkind = \"k2\"\nquantity = \"absolute\"\n\
         value = \"5.0\"\npolicy = \"p\"\nsuite = \"s\"\nruntime = \"rt\"\n\n\
         [[claim]]\nid = \"c\"\ntext = \"the ratio is 9.99\"\nevidence = \"derived\"\n\
         observations = [\"x\", \"y\"]\n\
         [claim.derivation]\ntransform = \"ratio\"\nnumerator = \"x\"\n\
         denominator = \"y\"\ndecimals = 2\n",
    );
    let e = ingest(&s, &text, false).unwrap_err();
    assert!(e.contains("2.00") || e.contains("Mismatch"), "{e}");
}

#[test]
fn an_unknown_alias_names_itself_and_lists_what_exists() {
    let s = Store::open_in_memory().unwrap();
    let e = ingest(
        &s,
        &doc(
            "[[observation]]\nid = \"o\"\nkind = \"k\"\nquantity = \"absolute\"\n\
             value = \"1.0\"\npolicy = \"nope\"\nsuite = \"s\"\nruntime = \"rt\"\n",
        ),
        false,
    )
    .unwrap_err();
    assert!(e.contains("\"nope\""), "{e}");
    assert!(
        e.contains("defined:"),
        "an error that does not say what IS valid: {e}"
    );
}

#[test]
fn an_unknown_field_is_rejected_rather_than_ignored() {
    // A typo'd key that is silently dropped is evidence quietly not recorded.
    let e = validate(&doc(
        "[[observation]]\nid = \"o\"\nkind = \"k\"\nquantity = \"absolute\"\n\
         value = \"1.0\"\npolicy = \"p\"\nsuite = \"s\"\nruntime = \"rt\"\n\
         referenc = \"typo\"\n",
    ))
    .unwrap_err();
    assert!(e.to_lowercase().contains("unknown"), "{e}");
}

#[test]
fn dry_run_counts_match_what_a_real_run_would_do() {
    // The point of a dry run is the count. Executing against a scratch database
    // would report already-present records as inserts, making the number wrong
    // precisely when the operator is relying on it.
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/contested.toml"
    ))
    .unwrap();
    let s = Store::open_in_memory().unwrap();
    ingest(&s, &text, false).unwrap(); // everything now present

    let dry = ingest(&s, &text, true).unwrap();
    assert_eq!(
        dry.inserted, 0,
        "a second dry run must predict zero inserts"
    );
    assert_eq!(dry.already_present, 12);
}

#[test]
fn dry_run_validates_everything_and_writes_nothing() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/contested.toml"
    ))
    .unwrap();
    let s = Store::open_in_memory().unwrap();
    let r = ingest(&s, &text, true).unwrap();
    assert!(r.dry_run && r.claims == 7);
    let n: i64 = s
        .conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0, "dry run must not write");

    // Polarity: a dry run still catches a dangling reference, or it would be a
    // check that passes everything.
    assert!(ingest(
        &s,
        &doc(
            "[[claim]]\nid = \"c\"\ntext = \"t\"\nevidence = \"external\"\n\
              observations = [\"ghost\"]\n"
        ),
        true
    )
    .is_err());
}

#[test]
fn an_unsupported_version_is_refused() {
    let e = validate("version = 2\nrecorded_at = \"2026-07-31T00:00:00Z\"\n").unwrap_err();
    assert!(e.contains("unsupported version 2"), "{e}");
}
