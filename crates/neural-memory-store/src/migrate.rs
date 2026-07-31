//! Versioned, forward-only migrations.
//!
//! Migrations are embedded with `include_str!` rather than read from disk, so a
//! built binary cannot disagree with the schema it was tested against.

use rusqlite::Connection;

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
        let already: i64 = conn.query_row(
            "SELECT count(*) FROM schema_migrations WHERE version = ?1",
            [version],
            |r| r.get(0),
        )?;
        if already > 0 {
            continue;
        }
        // Each migration is one transaction: a failure leaves no partial schema.
        let tx = conn.transaction()?;
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
