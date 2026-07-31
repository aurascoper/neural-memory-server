//! Concurrency, measured rather than assumed.
//!
//! This is not hypothetical: the MCP server is registered user-scope, so every
//! Claude Code session spawns a process against one SQLite file. Concurrent
//! access is the default deployment.
//!
//! Assertions are on **invariants**, not timing — `recorded_seq` strictly
//! increasing, no partial ingest, no error escaping to the caller. A test that
//! asserts "this took under N ms" passes or fails with machine load and teaches
//! nothing. The mutation harness already demonstrated how convincingly a
//! timing-dependent instrument can lie.
//!
//! Each `Store::open` creates its own SQLite connection, which is what locking
//! is scoped to, so threads here exercise the real contention path. The
//! migration race additionally needs separate processes and lives in
//! `tests/concurrency_processes.rs`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use neural_memory_domain::*;
use neural_memory_store::*;

fn tmp() -> (tempfile::TempDir, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("store.db");
    (d, p)
}

fn write_claim(s: &Store, text: &str) -> Result<(String, Wrote), WriteError> {
    s.put_memory(
        WriteChannel::Operator,
        &MemoryWrite {
            terms: MemoryRecordTerms {
                claim: text.into(),
                evidence_class: EvidenceClass::ExternalClaim,
                source_artifact_sha256: None,
                source_locator: None,
                observation_identities: vec![],
                harness_run_id: None,
            },
            occurred_at: None,
            recorded_at: Some("2026-07-31T18:00:00Z"),
            derivation: None,
        },
    )
}

#[test]
fn concurrent_writers_all_land_and_the_sequence_never_reuses() {
    // Two agents in two sessions calling `remember` at the same moment. Both
    // are reachable from the MCP surface, so this is the ordinary case.
    let (_d, path) = tmp();
    Store::open(&path).unwrap(); // create + migrate once, uncontended

    const THREADS: usize = 8;
    const EACH: usize = 25;
    let barrier = Arc::new(Barrier::new(THREADS));
    let errors = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..THREADS)
        .map(|t| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            let errors = Arc::clone(&errors);
            thread::spawn(move || {
                let s = Store::open(&path).expect("open");
                barrier.wait(); // maximise overlap
                for i in 0..EACH {
                    if write_claim(&s, &format!("claim from thread {t} number {i}")).is_err() {
                        errors.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    let s = Store::open(&path).unwrap();
    assert_eq!(
        errors.load(Ordering::SeqCst),
        0,
        "no write may fail: SQLITE_BUSY must be absorbed by the busy timeout, \
         not surfaced to an agent that has no way to retry"
    );
    let n: i64 = s
        .conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        n as usize,
        THREADS * EACH,
        "every claim must land exactly once"
    );

    // The transaction-time axis must survive contention. `as_of` is built on
    // it, so reuse or non-monotonicity would silently corrupt belief
    // reconstruction rather than fail loudly.
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
        seqs.windows(2).all(|w| w[1] > w[0]),
        "recorded_seq must stay strictly increasing under contention"
    );
    assert_eq!(
        seqs.len(),
        seqs.iter().collect::<std::collections::BTreeSet<_>>().len(),
        "no sequence value may be reused"
    );
}

#[test]
fn a_reader_never_observes_a_half_applied_ingest() {
    // The atomicity fix is what makes this assertable. A reader must see the
    // document either wholly present or wholly absent -- never the claims
    // without their wiring, which is the state that looks complete and is not.
    let (_d, path) = tmp();
    Store::open(&path).unwrap();

    let doc = format!(
        "version = 1\nrecorded_at = \"2026-07-31T18:00:00Z\"\n\n\
         [[artifact]]\nid = \"a\"\nkind = \"doc\"\nsha256 = \"{}\"\n\
         bytes = 1\nmedia_type = \"text/plain\"\nuri = \"file:///a\"\n\n{}",
        "aa".repeat(32),
        (0..40)
            .map(|i| format!(
                "[[claim]]\nid = \"c{i}\"\ntext = \"bulk claim {i}\"\n\
                 evidence = \"external\"\n\n"
            ))
            .collect::<String>()
    );

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observations = Arc::new(std::sync::Mutex::new(Vec::<i64>::new()));
    let reader = {
        let path = path.clone();
        let stop = Arc::clone(&stop);
        let observations = Arc::clone(&observations);
        thread::spawn(move || {
            let s = Store::open(&path).unwrap();
            while !stop.load(Ordering::SeqCst) {
                let n: i64 = s
                    .conn
                    .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
                    .unwrap_or(-1);
                observations.lock().unwrap().push(n);
            }
        })
    };

    let s = Store::open(&path).unwrap();
    ingest::ingest(&s, &doc, false).expect("bulk ingest");
    stop.store(true, Ordering::SeqCst);
    reader.join().unwrap();

    let seen = observations.lock().unwrap();
    assert!(!seen.is_empty(), "the reader must actually have run");
    assert!(
        seen.iter().all(|&n| n == 0 || n == 40),
        "a reader saw a partial document: {:?}",
        seen.iter().collect::<std::collections::BTreeSet<_>>()
    );
    assert!(!seen.contains(&-1), "no read may error");
}

#[test]
fn the_busy_timeout_is_what_absorbs_write_contention() {
    // A negative control for `concurrent_writers_all_land_...`. That test
    // passing tells us nothing unless writes genuinely contend -- if a second
    // writer could always proceed regardless, it would be green for the wrong
    // reason, which is the same failure mode as a fixture that only asserts the
    // pass side.
    //
    // So: prove the contention exists by removing the timeout and watching the
    // write fail, then prove the timeout is what absorbs it.
    let (_d, path) = tmp();
    let a = Store::open(&path).unwrap();
    let b = Store::open(&path).unwrap();

    a.conn.execute_batch("BEGIN IMMEDIATE").unwrap(); // A holds the write lock

    b.conn
        .busy_timeout(std::time::Duration::from_millis(0))
        .unwrap();
    assert!(
        write_claim(&b, "must not get through").is_err(),
        "with the busy handler off, a second writer MUST fail while a write \
         transaction is held -- if it does not, these tests are not exercising \
         contention at all"
    );

    // Same situation, timeout restored: the write must now wait and succeed.
    b.conn
        .busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let releaser = thread::spawn(move || {
        thread::sleep(std::time::Duration::from_millis(300));
        a.conn.execute_batch("COMMIT").unwrap();
    });
    write_claim(&b, "waited its turn")
        .expect("the busy timeout must absorb contention rather than surface it");
    releaser.join().unwrap();
}

#[test]
fn journal_mode_is_wal_on_every_connection() {
    // WAL is the premise of every reader-during-writer guarantee here. It is a
    // property of the file, so a connection that failed to set it would still
    // read `wal` -- but a store created by a caller that never got the pragma
    // through would silently be in rollback-journal mode and readers would
    // block. Cheap to assert, and the cold-open race made it a live question.
    let (_d, path) = tmp();
    let s = Store::open(&path).unwrap();
    assert_eq!(s.journal_mode().unwrap(), "wal");
    let again = Store::open(&path).unwrap();
    assert_eq!(again.journal_mode().unwrap(), "wal");
}

#[test]
fn commits_are_fsynced() {
    // synchronous=FULL (2). Under NORMAL, WAL does not fsync at commit and a
    // transaction that returned success can be lost to power loss. The cost was
    // measured before choosing this -- see the note in `Store::init` -- and it
    // is asserted here so a later "small" pragma change cannot quietly trade
    // committed history for write latency.
    let (_d, path) = tmp();
    assert_eq!(Store::open(&path).unwrap().synchronous().unwrap(), 2);
}

#[test]
fn a_failed_write_under_contention_leaves_nothing_behind() {
    // Contention must not turn a rejected document into a partial one. The
    // dangling-alias failure that motivated the atomicity fix happened without
    // any contention at all; this checks the same guarantee holds with it.
    let (_d, path) = tmp();
    Store::open(&path).unwrap();

    let bad = "version = 1\nrecorded_at = \"2026-07-31T18:00:00Z\"\n\n\
         [[claim]]\nid = \"a\"\ntext = \"should not survive\"\nevidence = \"external\"\n\n\
         [[edge]]\nfrom = \"a\"\nto = \"ghost\"\nkind = \"supports\"\n"
        .to_string();

    let barrier = Arc::new(Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let path = path.clone();
            let bad = bad.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let s = Store::open(&path).unwrap();
                barrier.wait();
                if i % 2 == 0 {
                    assert!(ingest::ingest(&s, &bad, false).is_err());
                } else {
                    let _ = write_claim(&s, &format!("good claim {i}"));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    let s = Store::open(&path).unwrap();
    let bad_rows: i64 = s
        .conn
        .query_row(
            "SELECT count(*) FROM memories WHERE claim = 'should not survive'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bad_rows, 0, "a failed document must leave no trace");
    assert!(s.integrity_ok().unwrap());
}

#[test]
fn opening_the_same_store_many_times_concurrently_is_safe() {
    // Every session opens the store, and `Store::open` runs `apply_all` every
    // time. On an already-migrated store that is a read, but it still races.
    let (_d, path) = tmp();
    Store::open(&path).unwrap();

    let barrier = Arc::new(Barrier::new(12));
    let errors = Arc::new(AtomicUsize::new(0));
    let handles: Vec<_> = (0..12)
        .map(|_| {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            let errors = Arc::clone(&errors);
            thread::spawn(move || {
                barrier.wait();
                match Store::open(&path) {
                    Ok(s) => {
                        if migrate::current_version(&s.conn).unwrap() != migrate::latest_version() {
                            errors.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    Err(_) => {
                        errors.fetch_add(1, Ordering::SeqCst);
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(
        errors.load(Ordering::SeqCst),
        0,
        "concurrent opens must all succeed"
    );
}

#[test]
fn reads_stay_available_while_a_writer_holds_a_transaction() {
    // WAL's central promise. If this failed, one agent mid-ingest would block
    // every other session's recall.
    let (_d, path) = tmp();
    let writer = Store::open(&path).unwrap();
    write_claim(&writer, "pre-existing claim").unwrap();

    let tx = writer.conn.unchecked_transaction().unwrap();
    writer
        .conn
        .execute(
            "INSERT INTO memories (record_digest, claim, evidence_class)
             VALUES (?1, 'held open', 'externalClaim')",
            ["bb".repeat(32)],
        )
        .unwrap();

    // Reader on a separate connection, while the write transaction is open.
    let reader = Store::open(&path).unwrap();
    let n: i64 = reader
        .conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .expect("a reader must not be blocked by an open write transaction");
    assert_eq!(
        n, 1,
        "the reader sees the committed state, not the pending one"
    );

    tx.commit().unwrap();
    let after: i64 = reader
        .conn
        .query_row("SELECT count(*) FROM memories", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after, 2, "and sees the new row once it commits");
}
