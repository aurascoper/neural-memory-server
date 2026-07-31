//! SQLite-backed store. This crate owns all I/O; the domain crate owns meaning.
//!
//! One file, WAL mode, foreign keys on. No daemon, no port, no pinned server
//! config, no suspend/resume connection probe — all of which the SQLite choice
//! deletes rather than mitigates.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

pub mod backup;
pub mod derive;
pub mod entity;
pub mod ingest;
pub mod migrate;
pub mod retrieval;
pub mod temporal;
pub mod vector;
pub mod write;

pub use backup::{verify_replica, BackupError, BackupReport, Difference};
pub use derive::{Derivation, DerivationError};
pub use entity::EntityHit;
pub use ingest::{ingest, validate, IngestDoc, IngestReport};
pub use retrieval::{
    Branch, BranchCounts, ConflictObligation, Direction, EdgeDetail, Hit, ObservationDetail,
    ProvenanceStep, RecallOptions, RecallResult, RecordDetail,
};
pub use temporal::{BeliefAt, SeqAt, TimelineEntry};
pub use vector::{VectorError, VectorHit};
pub use write::{EvidenceRefusal, MemoryWrite, WriteChannel, WriteError, Wrote};

#[derive(Debug)]
pub enum StoreError {
    Sql(rusqlite::Error),
    Migration(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sql(e) => write!(f, "sqlite: {e}"),
            StoreError::Migration(m) => write!(f, "migration: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sql(e)
    }
}

/// Switch the file to WAL, tolerating a cold-open stampede.
///
/// `PRAGMA journal_mode=WAL` needs a brief exclusive lock and — unlike an
/// ordinary statement — **does not invoke the busy handler**, so the
/// `busy_timeout` rusqlite sets does not cover it. It returns `SQLITE_BUSY`
/// straight away instead of waiting.
///
/// That is not theoretical. The MCP server is registered user-scope, so several
/// Claude Code sessions can open one store simultaneously, and on a store that
/// does not exist yet they all try the switch at once. Measured with eight
/// processes on a fresh file: **three failed with `database is locked` on each
/// of three consecutive runs**, while `busy_timeout` read back as 5000 ms the
/// whole time. See `tests/concurrency_processes.rs`.
///
/// Two parts, and both are needed:
///
/// - **Check first.** journal_mode is a persistent property of the *file*, not
///   of the connection, so once any process has set WAL every later opener
///   reads `wal` and takes no lock at all. This makes the steady state — the
///   overwhelmingly common one — race-free rather than merely lucky.
/// - **Retry the cold case.** On first creation every process legitimately sees
///   `delete` and must contend. Retrying is the documented remedy; there is no
///   way to make the pragma itself wait.
///
/// An in-memory database reports `memory` and cannot be switched. It returns
/// `Ok` without changing anything, which ends the loop on the first pass —
/// the same outcome as before this function existed.
fn enable_wal(conn: &Connection) -> Result<(), StoreError> {
    /// ~2.5 s of cumulative backoff, well beyond the observed contention.
    const ATTEMPTS: u32 = 50;

    let mut last: Option<rusqlite::Error> = None;
    for attempt in 0..ATTEMPTS {
        let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
        if mode.eq_ignore_ascii_case("wal") {
            return Ok(());
        }
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(e) if is_busy(&e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(
                    2 * (u64::from(attempt) + 1),
                ));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(last.map_or_else(
        || StoreError::Migration("journal_mode=WAL did not take".into()),
        StoreError::Sql,
    ))
}

/// Is this the lock contention that a retry can clear?
fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked,
                ..
            },
            _
        )
    )
}

pub struct Store {
    pub conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Self::init(conn)
    }

    /// Open without creating and without migrating.
    ///
    /// A replica being verified must be inspected exactly as the backup left
    /// it. Running migrations on it would make the verification a comparison
    /// against something this process just modified, and `CREATE` flags would
    /// let a typo'd path silently produce an empty store that then compares as
    /// "everything missing" rather than "no such file".
    pub fn open_read_only(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        // foreign_keys is OFF by default in SQLite and is per-connection, not a
        // property of the file. Every REFERENCES clause in 0001 is inert without
        // this line, so it is set before anything else runs.
        conn.pragma_update(None, "foreign_keys", "ON")?;
        enable_wal(&conn)?;
        // FULL, not the usual WAL-mode NORMAL.
        //
        // Under NORMAL, WAL does not fsync at commit: a transaction that
        // returned success can be lost to **power loss or an OS crash** (a
        // process crash is safe either way — the WAL survives and the next
        // opener replays it). For an append-only evidence store, losing the
        // last committed records is not a performance trade-off, it is the
        // store failing at the one thing it claims to do.
        //
        // Measured on this host with `neural-memory-bench-durability`
        // (ext4 on NVMe, median of 5 rounds):
        //
        //   shape           NORMAL      FULL      cost
        //   singleCommit    66.9 us   941.6 us   14.1x
        //   batchedIngest   19.1 us    23.7 us    1.2x
        //
        // The ratio looks alarming and is the wrong figure to decide on. What
        // matters is that a single `remember` goes from 0.07 ms to 0.94 ms
        // inside an MCP round trip measured in seconds, and that bulk loading
        // already goes through batched ingest, which pays 1.2x because one
        // fsync amortises across the whole document. The shape the 14x would
        // punish — tens of thousands of individual commits — is not a shape
        // this store has.
        //
        // It is also not a hypothetical risk here: this runs on a handheld, on
        // battery, that suspends and resumes.
        conn.pragma_update(None, "synchronous", "FULL")?;
        let mut store = Self { conn };
        migrate::apply_all(&mut store.conn)?;
        Ok(store)
    }

    /// Reports the journal mode actually in force, so a caller can verify WAL
    /// rather than assume the pragma took.
    pub fn journal_mode(&self) -> Result<String, StoreError> {
        Ok(self
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))?
            .to_ascii_lowercase())
    }

    /// `PRAGMA synchronous` as an integer: 0 off, 1 normal, 2 full, 3 extra.
    /// Exposed so the durability guarantee is assertable rather than assumed.
    pub fn synchronous(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("PRAGMA synchronous", [], |r| r.get(0))?)
    }

    /// `PRAGMA integrity_check` — the whole backup-and-verify story for one file.
    pub fn integrity_ok(&self) -> Result<bool, StoreError> {
        let v: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        Ok(v == "ok")
    }

    /// Confirms `foreign_keys` is actually on for this connection.
    pub fn foreign_keys_enforced(&self) -> Result<bool, StoreError> {
        let v: i64 = self
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
        Ok(v == 1)
    }
}
