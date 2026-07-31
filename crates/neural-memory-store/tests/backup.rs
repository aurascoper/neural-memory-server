//! Backup, and proof that the verifier is not vacuous.
//!
//! Two halves, and the second is the one that matters. Asserting that a backup
//! verifies clean says nothing unless the verifier can also report a backup
//! that is wrong — a checker that returns "fine" unconditionally passes the
//! happy path too. So every positive case here is paired with a mutation of the
//! replica that must be caught.

use std::path::Path;

use neural_memory_store::*;

fn corpus() -> String {
    // Exercises every table the verifier compares by identity, so a regression
    // in any one of them shows up rather than being skipped for lack of rows.
    let mut d = String::from(
        r#"
version = 1
run_id = "backup-fixture"
recorded_at = "2026-07-31T09:00:00Z"

[[artifact]]
id = "doc"
kind = "report"
sha256 = "1111111111111111111111111111111111111111111111111111111111111111"
bytes = 4096
media_type = "text/markdown"
uri = "file:///fixture.md"

[[policy]]
id = "p"
metric = "tokensPerSecond"
aggregation = "median"
comparison_rule = "higherIsBetter"
unit = "tok/s"

[[suite]]
id = "s"
name = "fixture-suite"
case_texts = ["one", "two"]
tokenizer = "llama"
context_cap = 4096

[[entity]]
name = "Radeon 890M"
type = "hardware"
aliases = ["890M", "gfx1150"]
"#,
    );
    for i in 0..12 {
        d.push_str(&format!(
            r#"
[[observation]]
id = "o{i}"
kind = "decodeThroughput"
quantity = "absolute"
value = "{}.5"
policy = "p"
suite = "s"
runtime = "vulkan-radv"
artifact = "doc"

[[claim]]
id = "c{i}"
text = "Fixture claim {i} about the Radeon 890M decode path"
evidence = "observed"
artifact = "doc"
locator = "§{i}"
observations = ["o{i}"]
"#,
            20 + i
        ));
    }
    for i in 1..12 {
        d.push_str(&format!(
            "\n[[edge]]\nfrom = \"c{i}\"\nto = \"c{}\"\nkind = \"supports\"\n",
            i - 1
        ));
    }
    d
}

fn populated() -> (tempfile::TempDir, std::path::PathBuf, Store) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("store.db");
    let s = Store::open(&db).unwrap();
    ingest::ingest(&s, &corpus(), false).expect("fixture ingest");
    (dir, db, s)
}

#[test]
fn a_backup_round_trips_every_identity() {
    let (dir, _db, s) = populated();
    let dest = dir.path().join("backup.db");

    let report = s.backup_to(&dest).expect("backup");
    assert_eq!(report.records, 12);
    assert_eq!(report.observations, 12);
    assert_eq!(report.edges, 11);
    assert!(report.bytes > 0);

    let diffs = verify_replica(&s, &dest).expect("verify");
    assert!(
        diffs.is_empty(),
        "clean backup must verify clean: {diffs:?}"
    );

    // And the replica must be usable, not merely equal: recall on the restored
    // copy has to work, since "restore" means serving from it.
    let restored = Store::open(&dest).unwrap();
    let hits = restored
        .recall(&RecallOptions {
            query: "Radeon 890M decode",
            as_of: "2026-07-31T12:00:00Z",
            ..Default::default()
        })
        .expect("recall from the restored copy");
    assert!(!hits.hits.is_empty(), "a restored store must still answer");
}

#[test]
fn the_verifier_reports_a_record_the_backup_lost() {
    // The dangerous direction. A backup missing history is the exact failure
    // that must never pass silently.
    let (dir, _db, s) = populated();
    let dest = dir.path().join("backup.db");
    s.backup_to(&dest).unwrap();

    {
        let tamper = Store::open(&dest).unwrap();
        tamper
            .conn
            .execute(
                "DELETE FROM memories WHERE record_digest =
                   (SELECT record_digest FROM memories ORDER BY recorded_seq LIMIT 1)",
                [],
            )
            .unwrap();
    }

    let diffs = verify_replica(&s, &dest).unwrap();
    assert!(
        diffs
            .iter()
            .any(|d| matches!(d, Difference::Missing { kind: "record", .. })),
        "a deleted record must be reported as MISSING, got {diffs:?}"
    );
}

#[test]
fn the_verifier_reports_a_claim_that_was_altered() {
    // Same identity, different content. A byte-count or row-count check would
    // miss this entirely, which is why the comparison is on content.
    let (dir, _db, s) = populated();
    let dest = dir.path().join("backup.db");
    s.backup_to(&dest).unwrap();

    {
        let tamper = Store::open(&dest).unwrap();
        tamper
            .conn
            .execute(
                "UPDATE memories SET claim = 'quietly rewritten'
                 WHERE recorded_seq = (SELECT min(recorded_seq) FROM memories)",
                [],
            )
            .unwrap();
    }

    let diffs = verify_replica(&s, &dest).unwrap();
    assert!(
        diffs
            .iter()
            .any(|d| matches!(d, Difference::Diverged { kind: "record", .. })),
        "an altered claim must be reported as DIVERGED, got {diffs:?}"
    );
}

#[test]
fn the_verifier_reports_a_dropped_edge_and_a_dropped_observation() {
    let (dir, _db, s) = populated();
    let dest = dir.path().join("backup.db");
    s.backup_to(&dest).unwrap();

    {
        let tamper = Store::open(&dest).unwrap();
        tamper
            .conn
            .execute("DELETE FROM provenance_edges WHERE rowid = 1", [])
            .unwrap();
        // memory_observations first: the FK is ON DELETE CASCADE, so this also
        // proves the count-compared tables are not being skipped.
        tamper
            .conn
            .execute(
                "DELETE FROM observations WHERE identity =
                   (SELECT identity FROM observations ORDER BY identity LIMIT 1)",
                [],
            )
            .unwrap();
    }

    let diffs = verify_replica(&s, &dest).unwrap();
    assert!(
        diffs
            .iter()
            .any(|d| matches!(d, Difference::Missing { kind: "edge", .. })),
        "a dropped edge must be reported: {diffs:?}"
    );
    assert!(
        diffs.iter().any(|d| matches!(
            d,
            Difference::Missing {
                kind: "observation",
                ..
            }
        )),
        "a dropped observation must be reported: {diffs:?}"
    );
    assert!(
        diffs.iter().any(|d| matches!(
            d,
            Difference::Diverged { kind: "table", id, .. } if id == "memory_observations"
        )),
        "the cascade into memory_observations must be reported: {diffs:?}"
    );
}

#[test]
fn backup_refuses_to_overwrite_an_existing_file() {
    // Overwriting the previous backup destroys the only copy that predates
    // whatever prompted this one.
    let (dir, _db, s) = populated();
    let dest = dir.path().join("backup.db");
    s.backup_to(&dest).unwrap();
    assert!(matches!(
        s.backup_to(&dest),
        Err(BackupError::DestinationExists(_))
    ));
}

#[test]
fn copying_the_db_file_alone_loses_committed_history_and_backup_does_not() {
    // This is the claim the README used to get wrong, tested rather than
    // asserted. In WAL mode a commit lands in `store.db-wal` and stays there
    // until a checkpoint, so `cp store.db` can omit history that is fully
    // committed and durable. `VACUUM INTO` reads the same snapshot the database
    // does, WAL included.
    let (dir, db, s) = populated();

    let wal = db.with_extension("db-wal");
    assert!(
        wal.exists() && std::fs::metadata(&wal).unwrap().len() > 0,
        "precondition: the commits must still be sitting in the WAL, \
         otherwise this test proves nothing about checkpointing"
    );

    let naive = dir.path().join("naive-copy.db");
    std::fs::copy(&db, &naive).unwrap();

    let proper = dir.path().join("proper.db");
    s.backup_to(&proper).unwrap();

    let count = |p: &Path| -> i64 {
        Store::open_read_only(p)
            .ok()
            .and_then(|st| {
                st.conn
                    .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
                    .ok()
            })
            .unwrap_or(-1)
    };

    let naive_n = count(&naive);
    let proper_n = count(&proper);
    assert_eq!(proper_n, 12, "VACUUM INTO must capture the WAL contents");
    assert_ne!(
        naive_n, 12,
        "if a bare file copy captured everything, the documented reason for \
         using VACUUM INTO would not hold on this platform and the README \
         should say so instead"
    );
    assert!(
        verify_replica(&s, &proper).unwrap().is_empty(),
        "the proper backup must verify clean"
    );
}

#[test]
fn a_backup_taken_while_writes_are_in_flight_is_still_consistent() {
    // The live-store case. VACUUM INTO holds a read transaction, so the copy is
    // one snapshot; it may be missing writes that land after it starts, but it
    // must never be internally inconsistent -- a claim without its observation
    // wiring is worse than a claim that is absent.
    let (dir, db, s) = populated();
    let dest = dir.path().join("live.db");

    let writer = {
        let db = db.clone();
        std::thread::spawn(move || {
            let w = Store::open(&db).unwrap();
            for i in 0..60 {
                let doc = format!(
                    "version = 1\nrecorded_at = \"2026-07-31T09:30:00Z\"\n\n\
                     [[claim]]\nid = \"x\"\ntext = \"concurrent claim {i}\"\n\
                     evidence = \"external\"\n"
                );
                let _ = ingest::ingest(&w, &doc, false);
            }
        })
    };

    let report = s.backup_to(&dest).expect("backup during writes");
    writer.join().unwrap();

    let replica = Store::open_read_only(&dest).unwrap();
    let check: String = replica
        .conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");

    // Every claim in the snapshot must still have its wiring intact.
    let orphans: i64 = replica
        .conn
        .query_row(
            "SELECT count(*) FROM memory_observations mo
             LEFT JOIN memories m ON m.record_digest = mo.record_digest
             WHERE m.record_digest IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0, "the snapshot must not contain dangling wiring");
    assert!(report.records >= 12);
}
