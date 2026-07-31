//! The migration race, across real processes.
//!
//! Threads are not enough here. SQLite's locking is per-connection, but the
//! scenario being tested is per-*process*: N Claude Code sessions starting at
//! once against a store that needs migrating. The MCP server is registered
//! user-scope, so that is the first-run experience, not a corner case.
//!
//! The child is a normal `#[ignore]`d test in this same binary, re-invoked via
//! `current_exe`. It only reports what happened; the parent judges. A child
//! that decided its own verdict could pass by exiting early.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use neural_memory_store::{migrate, Store};

const CHILD_DB: &str = "NM_CONCURRENCY_CHILD_DB";

/// Spawned by `migration_race_on_cold_open`. Not a test on its own.
#[test]
#[ignore = "subprocess worker for migration_race_on_cold_open"]
fn migration_race_child() {
    let Ok(db) = std::env::var(CHILD_DB) else {
        return;
    };
    let db = PathBuf::from(db);

    // All children park on a gate file so they reach `Store::open` together.
    // Without it they start staggered by process spawn time and the race the
    // test exists to provoke never happens.
    let gate = db.with_extension("gate");
    while !gate.exists() {
        std::thread::yield_now();
    }

    match Store::open(&db) {
        Ok(s) => match migrate::current_version(&s.conn) {
            Ok(v) => println!("CHILD_OK version={v}"),
            Err(e) => println!("CHILD_ERR reading version: {e}"),
        },
        Err(e) => println!("CHILD_ERR {e}"),
    }
}

fn race_children(db: &Path, n: usize) -> Vec<String> {
    let gate = db.with_extension("gate");
    let exe = std::env::current_exe().expect("current_exe");

    let kids: Vec<_> = (0..n)
        .map(|_| {
            Command::new(&exe)
                .args([
                    "--exact",
                    "migration_race_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env(CHILD_DB, db)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn child")
        })
        .collect();

    // Give every child time to reach the gate before opening it.
    std::thread::sleep(Duration::from_millis(400));
    std::fs::write(&gate, b"go").unwrap();

    kids.into_iter()
        .map(|k| {
            let out = k.wait_with_output().expect("child");
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .find(|l| l.starts_with("CHILD_"))
                .unwrap_or("CHILD_MISSING no verdict line")
                .to_string()
        })
        .collect()
}

#[test]
fn migration_race_on_cold_open() {
    // The store does not exist yet, so every process runs the full migration
    // set. This is the first-run scenario verbatim.
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("cold.db");

    let verdicts = race_children(&db, 8);
    let failed: Vec<_> = verdicts
        .iter()
        .filter(|v| !v.starts_with("CHILD_OK"))
        .collect();
    assert!(
        failed.is_empty(),
        "opening an unmigrated store concurrently must not fail any caller; \
         {} of {} failed:\n  {}",
        failed.len(),
        verdicts.len(),
        verdicts.join("\n  ")
    );

    // Every survivor must agree on the schema version, and it must be current.
    let latest = migrate::latest_version();
    for v in &verdicts {
        assert_eq!(
            v,
            &format!("CHILD_OK version={latest}"),
            "a child saw a schema version other than {latest}"
        );
    }

    // And the store must be usable afterwards, not merely present.
    let s = Store::open(&db).unwrap();
    assert!(s.integrity_ok().unwrap());
    assert_eq!(migrate::current_version(&s.conn).unwrap(), latest);
    let applied: i64 = s
        .conn
        .query_row("SELECT count(*) FROM schema_migrations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        applied, latest,
        "each migration must be recorded exactly once, not once per racing process"
    );
}

#[test]
fn opening_an_already_migrated_store_concurrently_is_a_no_op() {
    // The steady state, and by far the more common one: the store exists and
    // is current, and eight sessions open it at once. `apply_all` still runs.
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("warm.db");
    Store::open(&db).unwrap();

    let verdicts = race_children(&db, 8);
    let latest = migrate::latest_version();
    for v in &verdicts {
        assert_eq!(v, &format!("CHILD_OK version={latest}"));
    }
}
