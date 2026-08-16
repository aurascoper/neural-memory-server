//! Local personal memory in its own SQLite database.
//!
//! This crate does not open or migrate the evidence store. Its schema is
//! embedded here and is intended for `personal.db` only.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

mod sync;
pub use sync::*;
pub mod embeddings;
pub mod evidence_dr;
pub mod interaction_logs_dr;
pub mod personal_mcp;
pub mod runtime;
pub mod transport;

pub const IDENTITY_DOMAIN: &str = "claude-mind.memory.v1";

#[derive(Debug)]
pub enum PersonalError {
    Sql(rusqlite::Error),
    Metadata(String),
    Conflict(String),
}

impl std::fmt::Display for PersonalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(e) => write!(f, "sqlite: {e}"),
            Self::Metadata(e) => write!(f, "metadata: {e}"),
            Self::Conflict(e) => write!(f, "conflict: {e}"),
        }
    }
}

impl std::error::Error for PersonalError {}

impl From<rusqlite::Error> for PersonalError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sighting {
    pub created_at: String,
    pub source: Option<String>,
    pub conversation: Option<String>,
    pub origin_device: String,
    pub origin_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture<'a> {
    pub content: &'a str,
    pub occurred_at: Option<&'a str>,
    pub metadata_json: &'a str,
    pub sighting: Sighting,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRecord {
    pub digest: String,
    pub content: String,
    pub occurred_at: Option<String>,
    pub metadata: String,
    pub created_at: String,
    pub sighting_count: u64,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub epoch: u64,
    pub sequence: u64,
    pub replicated_as_of: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalDivergence {
    pub id: String,
    pub digest_a: String,
    pub digest_b: String,
    pub status: String,
    pub created_at: String,
    pub acknowledged_at: Option<String>,
}

#[derive(Serialize)]
struct IdentityDocument<'a> {
    #[serde(rename = "contentDomain")]
    content_domain: &'static str,
    metadata: &'a Value,
    #[serde(rename = "occurredAt")]
    occurred_at: Option<&'a str>,
    text: &'a str,
}

enum JsonShape {
    Scalar,
    Array(Vec<JsonShape>),
    Object(Vec<(String, JsonShape)>),
}

impl<'de> Deserialize<'de> for JsonShape {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ShapeVisitor;

        impl<'de> Visitor<'de> for ShapeVisitor {
            type Value = JsonShape;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("JSON")
            }

            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(JsonShape::Scalar)
            }

            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(JsonShape::Scalar)
            }

            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(JsonShape::Scalar)
            }

            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(JsonShape::Scalar)
            }

            fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
                Ok(JsonShape::Scalar)
            }

            fn visit_string<E>(self, _: String) -> Result<Self::Value, E> {
                Ok(JsonShape::Scalar)
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(JsonShape::Scalar)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(JsonShape::Scalar)
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(JsonShape::Array(values))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut entries = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(JsonShape::Object(entries))
            }
        }

        deserializer.deserialize_any(ShapeVisitor)
    }
}

fn validate_metadata_keys(value: &JsonShape) -> Result<(), PersonalError> {
    match value {
        JsonShape::Scalar => Ok(()),
        JsonShape::Array(values) => values.iter().try_for_each(validate_metadata_keys),
        JsonShape::Object(entries) => {
            let mut exact = HashSet::new();
            let mut normalized = HashMap::new();
            for (key, _) in entries {
                if !exact.insert(key.as_str()) {
                    return Err(PersonalError::Metadata(format!(
                        "duplicate metadata key: {key:?}"
                    )));
                }
                let nfc: String = key.nfc().collect();
                if let Some(previous) = normalized.insert(nfc, key.as_str()) {
                    if previous != key {
                        return Err(PersonalError::Metadata(format!(
                            "metadata key NFC collision: {previous:?} and {key:?}"
                        )));
                    }
                }
            }
            for (key, value) in entries {
                if !key.nfc().eq(key.chars()) {
                    return Err(PersonalError::Metadata(format!(
                        "non-NFC metadata key: {key:?}"
                    )));
                }
                validate_metadata_keys(value)?;
            }
            Ok(())
        }
    }
}

fn parse_shape(input: &[u8]) -> Result<JsonShape, PersonalError> {
    serde_json::from_slice(input).map_err(|error| PersonalError::Metadata(error.to_string()))
}

fn preflight_metadata(input: &[u8]) -> Result<(), PersonalError> {
    validate_metadata_keys(&parse_shape(input)?)
}

fn validate_sync_metadata(value: &JsonShape) -> Result<(), PersonalError> {
    match value {
        JsonShape::Scalar => Ok(()),
        JsonShape::Array(values) => values.iter().try_for_each(validate_sync_metadata),
        JsonShape::Object(entries) => entries.iter().try_for_each(|(key, value)| {
            if key == "metadata" {
                validate_metadata_keys(value)
            } else {
                validate_sync_metadata(value)
            }
        }),
    }
}

fn preflight_sync_payload_metadata(input: &[u8]) -> Result<(), PersonalError> {
    validate_sync_metadata(&parse_shape(input)?)
}

fn parse_metadata(input: &str) -> Result<Value, PersonalError> {
    preflight_metadata(input.as_bytes())?;
    serde_json::from_str(input).map_err(|e| PersonalError::Metadata(e.to_string()))
}

fn canonicalize<T: Serialize>(value: &T) -> Result<Vec<u8>, PersonalError> {
    serde_json_canonicalizer::to_vec(value).map_err(|e| PersonalError::Metadata(e.to_string()))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn divergence_id(digest_a: &str, digest_b: &str) -> String {
    hex_sha256(format!("claude-mind.personal-divergence.v1\0{digest_a}\0{digest_b}").as_bytes())
}

pub fn canonical_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 24
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'.'
        || bytes[23] != b'Z'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, b)| !matches!(i, 4 | 7 | 10 | 13 | 16 | 19 | 23) && !b.is_ascii_digit())
    {
        return false;
    }
    let number = |start: usize, end: usize| {
        value[start..end]
            .parse::<u32>()
            .expect("digit ranges were checked")
    };
    let year = number(0, 4);
    let month = number(5, 7);
    let day = number(8, 10);
    let hour = number(11, 13);
    let minute = number(14, 16);
    let second = number(17, 19);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day) && hour < 24 && minute < 60 && second < 60
}

/// RFC 8785 identity document bytes shared with other implementations.
pub fn identity_bytes(
    content: &str,
    occurred_at: Option<&str>,
    metadata_json: &str,
) -> Result<Vec<u8>, PersonalError> {
    if occurred_at.is_some_and(|value| !canonical_timestamp(value)) {
        return Err(PersonalError::Metadata(
            "occurredAt must be UTC RFC3339 with exactly milliseconds".into(),
        ));
    }
    let normalized: String = content.nfc().collect();
    let metadata = parse_metadata(metadata_json)?;
    canonicalize(&IdentityDocument {
        content_domain: IDENTITY_DOMAIN,
        metadata: &metadata,
        occurred_at,
        text: &normalized,
    })
}

/// Return the digest, normalized text, and canonical metadata stored on disk.
pub fn identity(
    content: &str,
    occurred_at: Option<&str>,
    metadata_json: &str,
) -> Result<(String, String, String), PersonalError> {
    if occurred_at.is_some_and(|value| !canonical_timestamp(value)) {
        return Err(PersonalError::Metadata(
            "occurredAt must be UTC RFC3339 with exactly milliseconds".into(),
        ));
    }
    let normalized: String = content.nfc().collect();
    let metadata = parse_metadata(metadata_json)?;
    let canonical_metadata =
        String::from_utf8(canonicalize(&metadata)?).expect("canonical JSON is UTF-8");
    let document = IdentityDocument {
        content_domain: IDENTITY_DOMAIN,
        metadata: &metadata,
        occurred_at,
        text: &normalized,
    };
    Ok((
        hex_sha256(&canonicalize(&document)?),
        normalized,
        canonical_metadata,
    ))
}

pub struct PersonalStore {
    pub conn: Connection,
}

impl PersonalStore {
    pub fn open(path: &Path) -> Result<Self, PersonalError> {
        prepare_personal_database(path)?;
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, PersonalError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, PersonalError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS personal_schema_migrations (
                 version INTEGER PRIMARY KEY,
                 applied_at TEXT NOT NULL
             )",
        )?;
        let applied: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM personal_schema_migrations WHERE version = 1)",
            [],
            |row| row.get(0),
        )?;
        if !applied {
            tx.execute_batch(include_str!("../migrations/0001_init.sql"))?;
            tx.execute(
                "INSERT INTO personal_schema_migrations(version, applied_at)
                 VALUES (1, strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
                [],
            )?;
        }
        let embeddings_applied: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM personal_schema_migrations WHERE version = 2)",
            [],
            |row| row.get(0),
        )?;
        if !embeddings_applied {
            tx.execute_batch(include_str!("../migrations/0002_embeddings.sql"))?;
            tx.execute(
                "INSERT INTO personal_schema_migrations(version, applied_at)
                 VALUES (2, strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
                [],
            )?;
        }
        let embedding_failures_applied: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM personal_schema_migrations WHERE version = 3)",
            [],
            |row| row.get(0),
        )?;
        if !embedding_failures_applied {
            tx.execute_batch(include_str!("../migrations/0003_embedding_failures.sql"))?;
            tx.execute(
                "INSERT INTO personal_schema_migrations(version, applied_at)
                 VALUES (3, strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
                [],
            )?;
        }
        let divergences_applied: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM personal_schema_migrations WHERE version = 4)",
            [],
            |row| row.get(0),
        )?;
        if !divergences_applied {
            tx.execute_batch(include_str!("../migrations/0004_divergences.sql"))?;
            tx.execute(
                "INSERT INTO personal_schema_migrations(version, applied_at)
                 VALUES (4, strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
                [],
            )?;
        }
        let transition_fingerprint_applied: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM personal_schema_migrations WHERE version = 5)",
            [],
            |row| row.get(0),
        )?;
        if !transition_fingerprint_applied {
            tx.execute_batch(include_str!(
                "../migrations/0005_replica_transition_fingerprint.sql"
            ))?;
            tx.execute(
                "INSERT INTO personal_schema_migrations(version, applied_at)
                 VALUES (5, strftime('%Y-%m-%dT%H:%M:%SZ','now'))",
                [],
            )?;
        }
        tx.commit()?;
        Ok(Self { conn })
    }

    pub fn capture(&mut self, capture: &Capture<'_>) -> Result<String, PersonalError> {
        self.put(capture, true)
    }

    /// Merge a canonical record and its sighting without recording a local capture.
    pub fn merge_replica(&mut self, record: &Capture<'_>) -> Result<String, PersonalError> {
        self.put(record, false)
    }

    pub fn flag_divergence(
        &mut self,
        first: &str,
        second: &str,
        created_at: &str,
    ) -> Result<PersonalDivergence, PersonalError> {
        if !canonical_timestamp(created_at) {
            return Err(PersonalError::Metadata(
                "divergence createdAt must be canonical".into(),
            ));
        }
        if first == second {
            return Err(PersonalError::Conflict(
                "a record cannot diverge from itself".into(),
            ));
        }
        let (digest_a, digest_b) = if first < second {
            (first, second)
        } else {
            (second, first)
        };
        let id = divergence_id(digest_a, digest_b);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let records: i64 = tx.query_row(
            "SELECT count(*) FROM canonical_records WHERE digest IN (?1, ?2)",
            params![digest_a, digest_b],
            |row| row.get(0),
        )?;
        if records != 2 {
            return Err(PersonalError::Conflict(
                "both divergent records must exist".into(),
            ));
        }
        tx.execute(
            "INSERT OR IGNORE INTO personal_divergences
                 (id, digest_a, digest_b, status, created_at)
             VALUES (?1, ?2, ?3, 'unacknowledged', ?4)",
            params![id, digest_a, digest_b, created_at],
        )?;
        let divergence = tx.query_row(
            "SELECT id,digest_a,digest_b,status,created_at,acknowledged_at
             FROM personal_divergences WHERE id=?1",
            [&id],
            divergence_from_row,
        )?;
        tx.commit()?;
        Ok(divergence)
    }

    pub fn acknowledge_divergence(
        &self,
        id: &str,
        acknowledged_at: &str,
    ) -> Result<bool, PersonalError> {
        if !canonical_timestamp(acknowledged_at) {
            return Err(PersonalError::Metadata(
                "divergence acknowledgedAt must be canonical".into(),
            ));
        }
        Ok(self.conn.execute(
            "UPDATE personal_divergences
                SET status='acknowledged', acknowledged_at=?1
              WHERE id=?2 AND status='unacknowledged'",
            params![acknowledged_at, id],
        )? == 1)
    }

    pub fn list_unacknowledged_divergences(
        &self,
    ) -> Result<Vec<PersonalDivergence>, PersonalError> {
        let mut statement = self.conn.prepare(
            "SELECT id,digest_a,digest_b,status,created_at,acknowledged_at
             FROM personal_divergences WHERE status='unacknowledged'
             ORDER BY created_at,id",
        )?;
        let divergences = statement
            .query_map([], divergence_from_row)?
            .collect::<Result<_, _>>()?;
        Ok(divergences)
    }

    pub fn reset_replica_for_reenrollment(
        &mut self,
        expected_predecessor_epoch: u64,
    ) -> Result<usize, PersonalError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_epoch: u64 = tx.query_row(
            "SELECT epoch FROM replica_cursor WHERE singleton=1",
            [],
            |row| row.get::<_, i64>(0).map(|value| value as u64),
        )?;
        if current_epoch != expected_predecessor_epoch {
            return Err(PersonalError::Conflict(
                "replica predecessor epoch mismatch".into(),
            ));
        }
        let replica_only: Vec<String> = {
            let mut statement = tx.prepare(
                "SELECT r.record_digest FROM replica_records r
                 WHERE NOT EXISTS (SELECT 1 FROM captures c WHERE c.record_digest=r.record_digest)
                 ORDER BY r.record_digest",
            )?;
            let values = statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            values
        };
        for digest in &replica_only {
            tx.execute(
                "DELETE FROM personal_divergences WHERE digest_a=?1 OR digest_b=?1",
                [digest],
            )?;
            tx.execute(
                "DELETE FROM personal_embedding_queue WHERE record_digest=?1",
                [digest],
            )?;
            tx.execute(
                "DELETE FROM personal_embeddings WHERE record_digest=?1",
                [digest],
            )?;
            tx.execute(
                "DELETE FROM personal_embedding_failures WHERE record_digest=?1",
                [digest],
            )?;
            tx.execute("DELETE FROM record_tags WHERE record_digest=?1", [digest])?;
            tx.execute("DELETE FROM sightings WHERE record_digest=?1", [digest])?;
        }
        tx.execute("DELETE FROM replica_records", [])?;
        for digest in &replica_only {
            tx.execute("DELETE FROM canonical_records WHERE digest=?1", [digest])?;
        }
        tx.execute(
            "UPDATE replica_cursor
             SET epoch=?1,sequence=0,replicated_as_of=NULL,transition_fingerprint=NULL
             WHERE singleton=1",
            [expected_predecessor_epoch as i64],
        )?;
        tx.commit()?;
        Ok(replica_only.len())
    }

    pub fn bump_promotion_epoch_and_snapshot(
        &mut self,
        created_at: &str,
    ) -> Result<SyncCursor, PersonalError> {
        if !canonical_timestamp(created_at) {
            return Err(PersonalError::Metadata("noncanonical snapshot time".into()));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let epoch: i64 = tx.query_row(
            "UPDATE promotion_state SET epoch=epoch+1,sequence=0 WHERE singleton=1 RETURNING epoch",
            [],
            |row| row.get(0),
        )?;
        let digests: Vec<(String, bool)> = {
            let mut statement = tx.prepare(
                "SELECT DISTINCT r.digest,r.tombstoned != 0
                 FROM canonical_records r JOIN captures c ON c.record_digest=r.digest
                 ORDER BY r.digest",
            )?;
            let values = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;
            values
        };
        for (index, (digest, tombstoned)) in digests.iter().enumerate() {
            let sequence = index as i64 + 1;
            tx.execute(
                "INSERT INTO promotion_changes(epoch,sequence,operation,record_digest)
                 VALUES (?1,?2,?3,?4)",
                params![
                    epoch,
                    sequence,
                    if *tombstoned { "tombstone" } else { "upsert" },
                    digest
                ],
            )?;
            tx.execute(
                "INSERT INTO promotion_outbox(epoch,sequence,record_digest,enqueued_at)
                 VALUES (?1,?2,?3,?4)",
                params![epoch, sequence, digest, created_at],
            )?;
        }
        tx.execute(
            "UPDATE promotion_state SET sequence=?1 WHERE singleton=1",
            [digests.len() as i64],
        )?;
        tx.commit()?;
        Ok(SyncCursor {
            epoch: epoch as u64,
            sequence: digests.len() as u64,
        })
    }

    fn put(&mut self, capture: &Capture<'_>, local: bool) -> Result<String, PersonalError> {
        if !canonical_timestamp(&capture.sighting.created_at) {
            return Err(PersonalError::Metadata(
                "capturedAt must be UTC RFC3339 with exactly milliseconds".into(),
            ));
        }
        let (digest, content, metadata) =
            identity(capture.content, capture.occurred_at, capture.metadata_json)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR IGNORE INTO canonical_records
                 (digest, identity_domain, content, occurred_at, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                digest,
                IDENTITY_DOMAIN,
                content,
                capture.occurred_at,
                metadata,
                capture.sighting.created_at,
            ],
        )?;
        tx.execute(
            "UPDATE canonical_records SET created_at = min(created_at, ?1)
             WHERE digest = ?2",
            params![capture.sighting.created_at, digest],
        )?;

        let existing_digest: Option<String> = tx
            .query_row(
                "SELECT record_digest FROM sightings
                 WHERE origin_device = ?1 AND origin_id = ?2",
                params![capture.sighting.origin_device, capture.sighting.origin_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing_digest {
            if existing != digest {
                return Err(PersonalError::Conflict(format!(
                    "origin {}/{} already identifies {existing}",
                    capture.sighting.origin_device, capture.sighting.origin_id
                )));
            }
        }

        tx.execute(
            "INSERT OR IGNORE INTO sightings
                 (origin_device, origin_id, record_digest, created_at, source, conversation)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                capture.sighting.origin_device,
                capture.sighting.origin_id,
                digest,
                capture.sighting.created_at,
                capture.sighting.source,
                capture.sighting.conversation,
            ],
        )?;
        if local {
            let inserted = tx.execute(
                "INSERT OR IGNORE INTO captures
                     (origin_device, origin_id, record_digest, captured_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    capture.sighting.origin_device,
                    capture.sighting.origin_id,
                    digest,
                    capture.sighting.created_at,
                ],
            )? == 1;
            if inserted {
                let (epoch, sequence): (i64, i64) = tx.query_row(
                    "UPDATE promotion_state SET sequence = sequence + 1 WHERE singleton = 1
                     RETURNING epoch, sequence",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                tx.execute(
                    "INSERT INTO promotion_changes(epoch, sequence, operation, record_digest)
                     VALUES (?1, ?2, 'upsert', ?3)",
                    params![epoch, sequence, digest],
                )?;
                tx.execute(
                    "INSERT INTO promotion_outbox(epoch, sequence, record_digest, enqueued_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![epoch, sequence, digest, capture.sighting.created_at],
                )?;
            }
        } else {
            tx.execute(
                "INSERT OR IGNORE INTO replica_records(record_digest) VALUES (?1)",
                [&digest],
            )?;
        }
        for tag in capture.tags.iter().filter(|tag| !tag.trim().is_empty()) {
            tx.execute("INSERT OR IGNORE INTO tags(name) VALUES (?1)", [tag])?;
            tx.execute(
                "INSERT OR IGNORE INTO record_tags(record_digest, tag) VALUES (?1, ?2)",
                params![digest, tag],
            )?;
        }
        tx.execute(
            "INSERT OR IGNORE INTO personal_embedding_queue(record_digest, profile_identity, enqueued_at)
             SELECT ?1, (SELECT profile_identity FROM personal_active_embedding_profile WHERE singleton = 1), ?2
             WHERE EXISTS (SELECT 1 FROM canonical_records WHERE digest = ?1 AND tombstoned = 0)",
            params![digest, capture.sighting.created_at],
        )?;
        tx.commit()?;
        Ok(digest)
    }

    pub fn get(&self, digest: &str) -> Result<Option<StoredRecord>, PersonalError> {
        let record = self
            .conn
            .query_row(
                "SELECT r.digest, r.content, r.occurred_at, r.metadata,
                        r.created_at, count(*)
                   FROM canonical_records r JOIN sightings s ON s.record_digest = r.digest
                  WHERE r.digest = ?1 GROUP BY r.digest",
                [digest],
                |row| {
                    Ok(StoredRecord {
                        digest: row.get(0)?,
                        content: row.get(1)?,
                        occurred_at: row.get(2)?,
                        metadata: row.get(3)?,
                        created_at: row.get(4)?,
                        sighting_count: row.get::<_, i64>(5)? as u64,
                        tags: Vec::new(),
                    })
                },
            )
            .optional()?;
        let Some(mut record) = record else {
            return Ok(None);
        };
        let mut stmt = self
            .conn
            .prepare("SELECT tag FROM record_tags WHERE record_digest = ?1 ORDER BY tag")?;
        record.tags = stmt
            .query_map([digest], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(record))
    }

    pub fn forget(&mut self, digest: &str, forgotten_at: &str) -> Result<bool, PersonalError> {
        if !canonical_timestamp(forgotten_at) {
            return Err(PersonalError::Metadata(
                "forgottenAt must be UTC RFC3339 with exactly milliseconds".into(),
            ));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE canonical_records SET tombstoned = 1
             WHERE digest = ?1 AND tombstoned = 0",
            [digest],
        )? == 1;
        if changed {
            tx.execute(
                "DELETE FROM personal_embedding_queue WHERE record_digest = ?1",
                [digest],
            )?;
            tx.execute(
                "DELETE FROM personal_embeddings WHERE record_digest = ?1",
                [digest],
            )?;
            tx.execute(
                "DELETE FROM personal_embedding_failures WHERE record_digest = ?1",
                [digest],
            )?;
            let (epoch, sequence): (i64, i64) = tx.query_row(
                "UPDATE promotion_state SET sequence = sequence + 1 WHERE singleton = 1
                 RETURNING epoch, sequence",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            tx.execute(
                "INSERT INTO promotion_changes(epoch, sequence, operation, record_digest)
                 VALUES (?1, ?2, 'tombstone', ?3)",
                params![epoch, sequence, digest],
            )?;
            tx.execute(
                "INSERT INTO promotion_outbox(epoch, sequence, record_digest, enqueued_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![epoch, sequence, digest, forgotten_at],
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn cursor(&self) -> Result<Cursor, PersonalError> {
        Ok(self.conn.query_row(
            "SELECT epoch, sequence, replicated_as_of FROM replica_cursor WHERE singleton = 1",
            [],
            |row| {
                Ok(Cursor {
                    epoch: row.get::<_, i64>(0)? as u64,
                    sequence: row.get::<_, i64>(1)? as u64,
                    replicated_as_of: row.get(2)?,
                })
            },
        )?)
    }

    /// Advance only when `(epoch, sequence)` increases lexicographically.
    pub fn advance_cursor(&self, next: &Cursor) -> Result<bool, PersonalError> {
        let changed = self.conn.execute(
            "UPDATE replica_cursor
                SET epoch = ?1, sequence = ?2, replicated_as_of = ?3
              WHERE singleton = 1
                AND (epoch < ?1 OR (epoch = ?1 AND sequence < ?2))",
            params![
                next.epoch as i64,
                next.sequence as i64,
                next.replicated_as_of
            ],
        )?;
        Ok(changed == 1)
    }
}

fn divergence_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PersonalDivergence> {
    Ok(PersonalDivergence {
        id: row.get(0)?,
        digest_a: row.get(1)?,
        digest_b: row.get(2)?,
        status: row.get(3)?,
        created_at: row.get(4)?,
        acknowledged_at: row.get(5)?,
    })
}

fn prepare_personal_database(path: &Path) -> Result<(), PersonalError> {
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| {
                PersonalError::Metadata(format!(
                    "create private personal database {}: {error}",
                    path.display()
                ))
            })?,
        Err(error) => {
            return Err(PersonalError::Metadata(format!(
                "open private personal database {}: {error}",
                path.display()
            )))
        }
    };
    let metadata = file.metadata().map_err(|error| {
        PersonalError::Metadata(format!(
            "inspect personal database {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.file_type().is_file() {
        return Err(PersonalError::Metadata(
            "personal database must be a regular file".into(),
        ));
    }
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(PersonalError::Metadata(
            "personal database must have mode 0600".into(),
        ));
    }
    // SAFETY: geteuid has no preconditions and does not dereference pointers.
    let expected_uid = unsafe { libc::geteuid() };
    if metadata.uid() != expected_uid {
        return Err(PersonalError::Metadata(format!(
            "personal database must be owned by effective user {expected_uid}"
        )));
    }
    Ok(())
}
