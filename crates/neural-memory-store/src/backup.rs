//! Backup, and the verification without which a backup is a rumour.
//!
//! ## Why not `cp`
//!
//! The README used to say backup is copying the file. That is wrong while the
//! store is open. In WAL mode committed data lives in `store.db-wal` until a
//! checkpoint folds it back, so a `cp` of `store.db` alone silently omits every
//! recent commit, and a `cp` taken mid-write is torn. The failure is quiet in
//! exactly the way that matters: the copy opens, passes `integrity_check`, and
//! is simply missing history.
//!
//! `VACUUM INTO` is the supported answer. It runs inside a read transaction, so
//! it sees one consistent snapshot including the WAL, and it writes a fresh,
//! defragmented database rather than a byte copy.
//!
//! ## Why verification is not optional
//!
//! A backup test that asserts the destination file exists is the same class of
//! error as a fixture that only asserts the pass side: it passes for a backup
//! that is empty, truncated, or a snapshot of the wrong database. So
//! [`verify_replica`] compares content — every `record_digest`, every
//! observation identity, every provenance edge — and the CLI runs it by default.
//!
//! ## What this does not protect against
//!
//! A backup on the same disk is a copy, not redundancy. It restores from a
//! mistaken write or a corrupted page; it does not survive the drive. Saying so
//! is part of documenting the guarantee.

use std::collections::BTreeSet;
use std::path::Path;

use crate::{Store, StoreError};

#[derive(Debug)]
pub enum BackupError {
    /// The destination already exists. `VACUUM INTO` refuses to overwrite and
    /// so does this — a backup that silently replaces an older one removes the
    /// only copy that predates whatever went wrong.
    DestinationExists(String),
    Store(StoreError),
    Sql(rusqlite::Error),
    /// The copy was written but does not match the source.
    Mismatch(Vec<Difference>),
}

impl std::fmt::Display for BackupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackupError::DestinationExists(p) => {
                write!(f, "destination already exists, refusing to overwrite: {p}")
            }
            BackupError::Store(e) => write!(f, "{e}"),
            BackupError::Sql(e) => write!(f, "sqlite: {e}"),
            BackupError::Mismatch(d) => {
                write!(f, "backup does not match source ({} differences)", d.len())?;
                for x in d.iter().take(10) {
                    write!(f, "\n  {x}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for BackupError {}

impl From<StoreError> for BackupError {
    fn from(e: StoreError) -> Self {
        BackupError::Store(e)
    }
}

impl From<rusqlite::Error> for BackupError {
    fn from(e: rusqlite::Error) -> Self {
        BackupError::Sql(e)
    }
}

/// One way the replica disagrees with the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Difference {
    /// Present in the source, absent from the replica. The dangerous direction:
    /// this is lost history.
    Missing { kind: &'static str, id: String },
    /// Present in the replica, absent from the source. Means the destination
    /// was not a clean snapshot of this database.
    Extra { kind: &'static str, id: String },
    /// Same identity, different content.
    Diverged {
        kind: &'static str,
        id: String,
        detail: String,
    },
    /// The replica failed `PRAGMA integrity_check`.
    Corrupt(String),
    /// Schema versions disagree, so a comparison of rows would be comparing
    /// different things.
    SchemaVersion { source: i64, replica: i64 },
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Difference::Missing { kind, id } => write!(f, "MISSING {kind} {id}"),
            Difference::Extra { kind, id } => write!(f, "EXTRA {kind} {id}"),
            Difference::Diverged { kind, id, detail } => {
                write!(f, "DIVERGED {kind} {id}: {detail}")
            }
            Difference::Corrupt(m) => write!(f, "CORRUPT replica: {m}"),
            Difference::SchemaVersion { source, replica } => {
                write!(f, "SCHEMA source at {source}, replica at {replica}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackupReport {
    pub destination: String,
    pub bytes: u64,
    pub records: usize,
    pub observations: usize,
    pub edges: usize,
    pub schema_version: i64,
}

/// Tables whose identity is compared row by row. These carry the doctrine: a
/// claim, what it rests on, and how it is wired. Losing any of them loses the
/// thing the store exists to preserve.
///
/// `(kind, table, identity expression, content expression)`
const IDENTIFIED: &[(&str, &str, &str, &str)] = &[
    (
        "record",
        "memories",
        "record_digest",
        "claim || x'01' || evidence_class || x'01' || coalesce(source_artifact_sha256,'')
         || x'01' || coalesce(source_locator,'') || x'01' || coalesce(harness_run_id,'')
         || x'01' || coalesce(runtime_identity,'') || x'01' || coalesce(occurred_at,'')
         || x'01' || coalesce(recorded_at,'') || x'01' || coalesce(superseded_by,'')
         || x'01' || coalesce(superseded_at,'') || x'01' || coalesce(retracted_at,'')
         || x'01' || coalesce(retraction_reason,'') || x'01' || metadata",
    ),
    (
        "observation",
        "observations",
        "identity",
        "observation_kind || x'01' || quantity_kind || x'01' || value_text
         || x'01' || coalesce(cast(value_real as text),'')
         || x'01' || measurement_policy_identity || x'01' || evaluation_suite_identity
         || x'01' || coalesce(reference_execution_identity,'') || x'01' || runtime_identity
         || x'01' || coalesce(artifact_sha256,'') || x'01' || observed_at",
    ),
    (
        "edge",
        "provenance_edges",
        "src_digest || x'01' || dst_digest || x'01' || edge_kind",
        "created_at",
    ),
    (
        "artifact",
        "artifacts",
        "sha256_hex",
        "artifact_kind || x'01' || cast(byte_size as text) || x'01' || media_type
         || x'01' || source_uri || x'01' || ingested_at",
    ),
    (
        "entity",
        "entities",
        "id",
        "canonical_name || x'01' || entity_type || x'01' || aliases",
    ),
];

impl Store {
    /// Write a consistent snapshot to `dest` using `VACUUM INTO`.
    ///
    /// Safe on a live store: it takes a read transaction for the duration, so
    /// writers are held off but nothing is torn. On a store of a few hundred
    /// records that pause is imperceptible; on a much larger one it would be
    /// worth measuring rather than assuming.
    pub fn backup_to(&self, dest: &Path) -> Result<BackupReport, BackupError> {
        if dest.exists() {
            return Err(BackupError::DestinationExists(dest.display().to_string()));
        }
        // Bound parameter, not string interpolation: a path is caller data.
        self.conn
            .execute("VACUUM INTO ?1", [&dest.to_string_lossy().to_string()])?;

        let count = |t: &str| -> Result<usize, BackupError> {
            let n: i64 = self
                .conn
                .query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))?;
            Ok(n as usize)
        };
        Ok(BackupReport {
            destination: dest.display().to_string(),
            bytes: std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0),
            records: count("memories")?,
            observations: count("observations")?,
            edges: count("provenance_edges")?,
            schema_version: crate::migrate::current_version(&self.conn)?,
        })
    }
}

/// Does `replica` actually contain what `source` contains?
///
/// Opens the replica read-only and compares content, not file metadata. Returns
/// every difference found rather than the first, because "what did the backup
/// lose" is the question an operator is actually asking.
pub fn verify_replica(source: &Store, replica_path: &Path) -> Result<Vec<Difference>, BackupError> {
    let replica = Store::open_read_only(replica_path)?;
    let mut diffs = Vec::new();

    // Structural integrity first. Comparing rows out of a corrupt file would
    // produce differences whose cause is not the one being reported.
    let check: String = replica
        .conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if check != "ok" {
        diffs.push(Difference::Corrupt(check));
        return Ok(diffs);
    }

    let sv = crate::migrate::current_version(&source.conn)?;
    let rv = crate::migrate::current_version(&replica.conn)?;
    if sv != rv {
        diffs.push(Difference::SchemaVersion {
            source: sv,
            replica: rv,
        });
        return Ok(diffs);
    }

    for (kind, table, id_expr, content_expr) in IDENTIFIED {
        let load = |s: &Store| -> Result<Vec<(String, String)>, BackupError> {
            let sql = format!("SELECT {id_expr}, {content_expr} FROM {table}");
            let mut st = s.conn.prepare(&sql)?;
            let rows = st
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        };
        let a: std::collections::BTreeMap<_, _> = load(source)?.into_iter().collect();
        let b: std::collections::BTreeMap<_, _> = load(&replica)?.into_iter().collect();

        let ka: BTreeSet<_> = a.keys().collect();
        let kb: BTreeSet<_> = b.keys().collect();
        for id in ka.difference(&kb) {
            diffs.push(Difference::Missing {
                kind,
                id: (*id).clone(),
            });
        }
        for id in kb.difference(&ka) {
            diffs.push(Difference::Extra {
                kind,
                id: (*id).clone(),
            });
        }
        for id in ka.intersection(&kb) {
            if a[*id] != b[*id] {
                diffs.push(Difference::Diverged {
                    kind,
                    id: (*id).clone(),
                    detail: "content differs".into(),
                });
            }
        }
    }

    // The remaining tables are compared by row count. Naming every column of
    // every table here would rot the moment a migration adds one; a count
    // catches loss, which is the failure mode a backup has.
    for table in [
        "memory_observations",
        "mentions",
        "relations",
        "tags",
        "memory_tags",
        "measurement_policies",
        "evaluation_suites",
        "reference_executions",
        "embedding_profiles",
        "embeddings",
        "sessions",
        "session_emissions",
    ] {
        let n = |s: &Store| -> Result<i64, BackupError> {
            Ok(s.conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)
        };
        let (x, y) = (n(source)?, n(&replica)?);
        if x != y {
            diffs.push(Difference::Diverged {
                kind: "table",
                id: table.to_string(),
                detail: format!("{x} rows in source, {y} in replica"),
            });
        }
    }

    Ok(diffs)
}
