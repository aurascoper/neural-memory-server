//! Versioned, forward-only migrations.
//!
//! Migrations are embedded with `include_str!` rather than read from disk, so a
//! built binary cannot disagree with the schema it was tested against.

use rusqlite::{Connection, TransactionBehavior};

use crate::StoreError;

/// `(version, name, sql)`, applied in ascending order.
const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "init", include_str!("../migrations/0001_init.sql")),
    (
        2,
        "embeddings",
        include_str!("../migrations/0002_embeddings.sql"),
    ),
    (
        3,
        "temporal",
        include_str!("../migrations/0003_temporal.sql"),
    ),
];

/// Highest migration this build knows about. Exposed so tests track the list
/// rather than a magic number that goes stale the moment a migration is added.
pub fn latest_version() -> i64 {
    MIGRATIONS.iter().map(|(v, _, _)| *v).max().unwrap_or(0)
}

pub fn apply_all(conn: &mut Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)",
    )?;

    for (version, name, sql) in MIGRATIONS {
        // Cheap unlocked pre-check. The steady state is "everything already
        // applied", and that path must not take a write lock merely to discover
        // there is nothing to do -- every session start runs this.
        if applied(conn, *version)? {
            continue;
        }

        // Each migration is one transaction: a failure leaves no partial schema.
        //
        // IMMEDIATE, not the default DEFERRED. A deferred transaction takes its
        // write lock at the first write, which is the migration SQL itself, so
        // two processes could both pass the check above, both begin, and both
        // run the same DDL -- one wins and the other fails on an object that
        // now exists. That is not hypothetical: with eight processes opening a
        // fresh store, one run in three produced
        // `0002_embeddings: duplicate column name: identity`. It survived a
        // first clean run and only appeared on the second, which is exactly
        // why three consecutive runs were required rather than one.
        //
        // IMMEDIATE takes the write lock at BEGIN and *does* honour
        // `busy_timeout`, so the losers wait rather than fail.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Re-check inside the lock. The pre-check above is only an optimisation
        // and is worthless on its own -- the winner may have committed this very
        // migration between that read and this BEGIN.
        if applied(&tx, *version)? {
            tx.rollback()?;
            continue;
        }

        tx.execute_batch(sql)
            .map_err(|e| StoreError::Migration(format!("{version:04}_{name}: {e}")))?;
        tx.execute(
            "INSERT OR REPLACE INTO schema_migrations(version, applied_at)
             VALUES (?1, strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
            [version],
        )?;
        tx.commit()?;
    }
    Ok(())
}

fn applied(conn: &Connection, version: i64) -> Result<bool, StoreError> {
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM schema_migrations WHERE version = ?1",
        [version],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Highest applied migration version, or 0.
pub fn current_version(conn: &Connection) -> Result<i64, StoreError> {
    Ok(conn
        .query_row(
            "SELECT coalesce(max(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0))
}
