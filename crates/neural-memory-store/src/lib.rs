//! SQLite-backed store. This crate owns all I/O; the domain crate owns meaning.
//!
//! One file, WAL mode, foreign keys on. No daemon, no port, no pinned server
//! config, no suspend/resume connection probe — all of which the SQLite choice
//! deletes rather than mitigates.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

pub mod derive;
pub mod entity;
pub mod ingest;
pub mod migrate;
pub mod retrieval;
pub mod vector;
pub mod write;

pub use derive::{Derivation, DerivationError};
pub use entity::EntityHit;
pub use ingest::{ingest, validate, IngestDoc, IngestReport};
pub use retrieval::{
    Branch, BranchCounts, ConflictObligation, Direction, EdgeDetail, Hit, ObservationDetail,
    ProvenanceStep, RecallOptions, RecallResult, RecordDetail,
};
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

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        // foreign_keys is OFF by default in SQLite and is per-connection, not a
        // property of the file. Every REFERENCES clause in 0001 is inert without
        // this line, so it is set before anything else runs.
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let mut store = Self { conn };
        migrate::apply_all(&mut store.conn)?;
        Ok(store)
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
