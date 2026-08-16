use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    canonical_timestamp, identity, preflight_sync_payload_metadata, PersonalError, PersonalStore,
    IDENTITY_DOMAIN,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SyncCursor {
    pub epoch: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSighting {
    pub origin_device: String,
    #[serde(rename = "originRecordID")]
    pub origin_record_id: String,
    pub captured_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(rename = "conversationID")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRecord {
    pub content_domain: String,
    pub content_digest: String,
    pub text: String,
    pub occurred_at: Option<String>,
    pub metadata: Value,
    pub created_at: String,
    pub tombstoned: bool,
    pub tags: Vec<String>,
    pub sightings: Vec<SyncSighting>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncChange {
    pub cursor: SyncCursor,
    pub operation: String,
    pub record: SyncRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDivergence {
    pub digest_a: String,
    pub digest_b: String,
    pub status: String,
    pub created_at: String,
    pub acknowledged_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPayloadV1 {
    pub source_device: String,
    pub from_exclusive: SyncCursor,
    pub through: SyncCursor,
    pub generated_at: String,
    pub changes: Vec<SyncChange>,
    #[serde(default)]
    pub divergences: Vec<SyncDivergence>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordTransition<'a> {
    source_device: &'a str,
    from_exclusive: SyncCursor,
    through: SyncCursor,
    changes: &'a [SyncChange],
}

fn transition_fingerprint(payload: &SyncPayloadV1) -> Result<String, PersonalError> {
    let transition = RecordTransition {
        source_device: &payload.source_device,
        from_exclusive: payload.from_exclusive,
        through: payload.through,
        changes: &payload.changes,
    };
    let canonical = serde_json_canonicalizer::to_string(&transition)
        .map_err(|error| PersonalError::Metadata(error.to_string()))?;
    Ok(crate::hex_sha256(canonical.as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SyncEnvelopeV1 {
    pub version: String,
    pub algorithm: String,
    #[serde(rename = "signerKeyID")]
    pub signer_key_id: String,
    pub payload_base64: String,
    pub signature_base64: String,
}

pub fn signer_key_id(key: &VerifyingKey) -> String {
    crate::hex_sha256(key.as_bytes())
}

pub fn sign_payload(
    payload: &SyncPayloadV1,
    key: &SigningKey,
) -> Result<SyncEnvelopeV1, PersonalError> {
    let bytes = serde_json::to_vec(payload).map_err(|e| PersonalError::Metadata(e.to_string()))?;
    let signature = key.sign(&bytes);
    Ok(SyncEnvelopeV1 {
        version: "SyncBundleV1".into(),
        algorithm: "Ed25519".into(),
        signer_key_id: signer_key_id(&key.verifying_key()),
        payload_base64: BASE64.encode(&bytes),
        signature_base64: BASE64.encode(signature.to_bytes()),
    })
}

/// Verify exact decoded payload bytes before parsing them as JSON.
pub fn verify_envelope(
    envelope: &SyncEnvelopeV1,
    enrolled_key: &VerifyingKey,
) -> Result<SyncPayloadV1, PersonalError> {
    if envelope.version != "SyncBundleV1"
        || envelope.algorithm != "Ed25519"
        || envelope.signer_key_id != signer_key_id(enrolled_key)
    {
        return Err(PersonalError::Conflict(
            "sync envelope identity mismatch".into(),
        ));
    }
    let payload = BASE64
        .decode(&envelope.payload_base64)
        .map_err(|e| PersonalError::Conflict(format!("payload base64: {e}")))?;
    let signature_bytes = BASE64
        .decode(&envelope.signature_base64)
        .map_err(|e| PersonalError::Conflict(format!("signature base64: {e}")))?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|e| PersonalError::Conflict(format!("signature: {e}")))?;
    enrolled_key
        .verify(&payload, &signature)
        .map_err(|_| PersonalError::Conflict("invalid sync signature".into()))?;
    preflight_sync_payload_metadata(&payload)?;
    serde_json::from_slice(&payload).map_err(|e| PersonalError::Metadata(e.to_string()))
}

fn successor(previous: SyncCursor, next: SyncCursor) -> bool {
    (next.epoch == previous.epoch && previous.sequence.checked_add(1) == Some(next.sequence))
        || (previous.epoch.checked_add(1) == Some(next.epoch) && next.sequence == 1)
}

fn validate_payload(payload: &SyncPayloadV1) -> Result<(), PersonalError> {
    if !canonical_timestamp(&payload.generated_at) {
        return Err(PersonalError::Metadata("noncanonical generatedAt".into()));
    }
    if let Some(first) = payload.changes.first() {
        let stream_epoch = first.cursor.epoch;
        let valid_epoch_shape = if stream_epoch == payload.from_exclusive.epoch {
            payload
                .changes
                .iter()
                .all(|change| change.cursor.epoch == stream_epoch)
        } else {
            payload.from_exclusive.epoch.checked_add(1) == Some(stream_epoch)
                && first.cursor.sequence == 1
                && payload
                    .changes
                    .iter()
                    .all(|change| change.cursor.epoch == stream_epoch)
        };
        if !valid_epoch_shape {
            return Err(PersonalError::Conflict(
                "mixed or skipped sync epoch".into(),
            ));
        }
    }
    let mut cursor = payload.from_exclusive;
    for change in &payload.changes {
        if !successor(cursor, change.cursor) {
            return Err(PersonalError::Conflict(
                "sync cursor gap or reversal".into(),
            ));
        }
        if !matches!(change.operation.as_str(), "upsert" | "tombstone")
            || (change.operation == "tombstone" && !change.record.tombstoned)
        {
            return Err(PersonalError::Conflict("invalid sync operation".into()));
        }
        validate_record(&change.record)?;
        cursor = change.cursor;
    }
    if cursor != payload.through {
        return Err(PersonalError::Conflict(
            "through cursor does not match changes".into(),
        ));
    }
    for divergence in &payload.divergences {
        let acknowledged = divergence.status == "acknowledged";
        if divergence.digest_a >= divergence.digest_b
            || !matches!(
                divergence.status.as_str(),
                "unacknowledged" | "acknowledged"
            )
            || !canonical_timestamp(&divergence.created_at)
            || divergence
                .acknowledged_at
                .as_deref()
                .is_some_and(|value| !canonical_timestamp(value))
            || acknowledged != divergence.acknowledged_at.is_some()
        {
            return Err(PersonalError::Conflict("invalid sync divergence".into()));
        }
    }
    Ok(())
}

fn validate_record(record: &SyncRecord) -> Result<(), PersonalError> {
    if record.content_domain != IDENTITY_DOMAIN
        || !canonical_timestamp(&record.created_at)
        || record
            .occurred_at
            .as_deref()
            .is_some_and(|value| !canonical_timestamp(value))
        || record
            .sightings
            .iter()
            .any(|sighting| !canonical_timestamp(&sighting.captured_at))
    {
        return Err(PersonalError::Metadata("invalid canonical record".into()));
    }
    let metadata = serde_json_canonicalizer::to_string(&record.metadata)
        .map_err(|e| PersonalError::Metadata(e.to_string()))?;
    let digest = identity(&record.text, record.occurred_at.as_deref(), &metadata)?.0;
    if digest != record.content_digest {
        return Err(PersonalError::Conflict("content digest mismatch".into()));
    }
    Ok(())
}

fn record_for(tx: &Transaction<'_>, digest: &str) -> Result<SyncRecord, PersonalError> {
    let mut record = tx.query_row(
        "SELECT content, occurred_at, metadata, created_at, tombstoned
         FROM canonical_records WHERE digest = ?1",
        [digest],
        |row| {
            let metadata: String = row.get(2)?;
            Ok(SyncRecord {
                content_domain: IDENTITY_DOMAIN.into(),
                content_digest: digest.into(),
                text: row.get(0)?,
                occurred_at: row.get(1)?,
                metadata: serde_json::from_str(&metadata).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        metadata.len(),
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                created_at: row.get(3)?,
                tombstoned: row.get::<_, i64>(4)? != 0,
                tags: Vec::new(),
                sightings: Vec::new(),
            })
        },
    )?;
    let mut tags =
        tx.prepare("SELECT tag FROM record_tags WHERE record_digest = ?1 ORDER BY tag")?;
    record.tags = tags
        .query_map([digest], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut sightings = tx.prepare(
        "SELECT origin_device, origin_id, created_at, source, conversation
         FROM sightings WHERE record_digest = ?1 ORDER BY origin_device, origin_id",
    )?;
    record.sightings = sightings
        .query_map([digest], |row| {
            Ok(SyncSighting {
                origin_device: row.get(0)?,
                origin_record_id: row.get(1)?,
                captured_at: row.get(2)?,
                source: row.get(3)?,
                conversation_id: row.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(record)
}

impl PersonalStore {
    pub fn export_promotions(
        &mut self,
        source_device: &str,
        from_exclusive: SyncCursor,
        generated_at: &str,
    ) -> Result<SyncPayloadV1, PersonalError> {
        if !canonical_timestamp(generated_at) {
            return Err(PersonalError::Metadata("noncanonical generatedAt".into()));
        }
        let tx = self.conn.transaction()?;
        let mut statement = tx.prepare(
            "SELECT epoch, sequence, operation, record_digest FROM promotion_changes
             WHERE epoch > ?1 OR (epoch = ?1 AND sequence > ?2)
             ORDER BY epoch, sequence",
        )?;
        let mut rows = statement
            .query_map(
                params![from_exclusive.epoch as i64, from_exclusive.sequence as i64],
                |row| {
                    Ok((
                        SyncCursor {
                            epoch: row.get::<_, i64>(0)? as u64,
                            sequence: row.get::<_, i64>(1)? as u64,
                        },
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(first_epoch) = rows.first().map(|row| row.0.epoch) {
            rows.truncate(
                rows.iter()
                    .take_while(|row| row.0.epoch == first_epoch)
                    .count(),
            );
        }
        let mut changes = Vec::with_capacity(rows.len());
        for (cursor, operation, digest) in rows {
            changes.push(SyncChange {
                cursor,
                operation,
                record: record_for(&tx, &digest)?,
            });
        }
        let through = changes
            .last()
            .map_or(from_exclusive, |change| change.cursor);
        drop(statement);
        let head = tx.query_row(
            "SELECT epoch,sequence FROM promotion_state WHERE singleton=1",
            [],
            |row| {
                Ok(SyncCursor {
                    epoch: row.get::<_, i64>(0)? as u64,
                    sequence: row.get::<_, i64>(1)? as u64,
                })
            },
        )?;
        let divergences = if through == head {
            let mut divergence_statement = tx.prepare(
                "SELECT digest_a,digest_b,status,created_at,acknowledged_at
                 FROM personal_divergences ORDER BY digest_a,digest_b",
            )?;
            let values = divergence_statement
                .query_map([], |row| {
                    Ok(SyncDivergence {
                        digest_a: row.get(0)?,
                        digest_b: row.get(1)?,
                        status: row.get(2)?,
                        created_at: row.get(3)?,
                        acknowledged_at: row.get(4)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            values
        } else {
            Vec::new()
        };
        tx.commit()?;
        Ok(SyncPayloadV1 {
            source_device: source_device.into(),
            from_exclusive,
            through,
            generated_at: generated_at.into(),
            changes,
            divergences,
        })
    }

    pub fn acknowledge_promotions(
        &self,
        through: SyncCursor,
        acknowledged_at: &str,
    ) -> Result<usize, PersonalError> {
        if !canonical_timestamp(acknowledged_at) {
            return Err(PersonalError::Metadata(
                "noncanonical acknowledgedAt".into(),
            ));
        }
        let allocated: SyncCursor = self.conn.query_row(
            "SELECT epoch, sequence FROM promotion_state WHERE singleton = 1",
            [],
            |row| {
                Ok(SyncCursor {
                    epoch: row.get::<_, i64>(0)? as u64,
                    sequence: row.get::<_, i64>(1)? as u64,
                })
            },
        )?;
        if through > allocated {
            return Err(PersonalError::Conflict(
                "ack exceeds allocated cursor".into(),
            ));
        }
        Ok(self.conn.execute(
            "UPDATE promotion_outbox SET status = 'promoted', promoted_at = ?1
             WHERE status = 'pending' AND (epoch < ?2 OR (epoch = ?2 AND sequence <= ?3))",
            params![
                acknowledged_at,
                through.epoch as i64,
                through.sequence as i64
            ],
        )?)
    }

    pub fn import_verified_bundle(
        &mut self,
        envelope: &SyncEnvelopeV1,
        enrolled_key: &VerifyingKey,
    ) -> Result<bool, PersonalError> {
        let payload = verify_envelope(envelope, enrolled_key)?;
        validate_payload(&payload)?;
        let fingerprint = (!payload.changes.is_empty())
            .then(|| transition_fingerprint(&payload))
            .transpose()?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (current_sync, accepted_fingerprint): (SyncCursor, Option<String>) = tx.query_row(
            "SELECT epoch,sequence,transition_fingerprint FROM replica_cursor WHERE singleton=1",
            [],
            |row| {
                Ok((
                    SyncCursor {
                        epoch: row.get::<_, i64>(0)? as u64,
                        sequence: row.get::<_, i64>(1)? as u64,
                    },
                    row.get(2)?,
                ))
            },
        )?;
        if payload.through < current_sync {
            return Ok(false);
        }
        if payload.through > current_sync && payload.from_exclusive != current_sync {
            return Err(PersonalError::Conflict(
                "bundle does not start at replica cursor".into(),
            ));
        }
        if payload.through == current_sync
            && fingerprint
                .as_ref()
                .is_some_and(|value| accepted_fingerprint.as_ref() != Some(value))
        {
            return Err(PersonalError::Conflict(
                "same-cursor record transition equivocation".into(),
            ));
        }
        for change in &payload.changes {
            if payload.through == current_sync {
                break;
            }
            merge_record(&tx, &change.record)?;
            if change.record.tombstoned {
                tx.execute(
                    "DELETE FROM personal_embedding_queue WHERE record_digest = ?1",
                    [&change.record.content_digest],
                )?;
                tx.execute(
                    "DELETE FROM personal_embeddings WHERE record_digest = ?1",
                    [&change.record.content_digest],
                )?;
                tx.execute(
                    "DELETE FROM personal_embedding_failures WHERE record_digest = ?1",
                    [&change.record.content_digest],
                )?;
            } else {
                tx.execute(
                    "INSERT OR IGNORE INTO personal_embedding_queue(record_digest, profile_identity, enqueued_at)
                     VALUES (?1, (SELECT profile_identity FROM personal_active_embedding_profile WHERE singleton = 1), ?2)",
                    params![change.record.content_digest, payload.generated_at],
                )?;
            }
        }
        let mut divergence_changed = false;
        for divergence in &payload.divergences {
            divergence_changed |= merge_divergence(&tx, divergence)?;
        }
        if payload.through > current_sync {
            tx.execute(
                "UPDATE replica_cursor
                 SET epoch = ?1, sequence = ?2, replicated_as_of = ?3,
                     transition_fingerprint = ?4
             WHERE singleton = 1",
                params![
                    payload.through.epoch as i64,
                    payload.through.sequence as i64,
                    payload.generated_at,
                    fingerprint
                ],
            )?;
        }
        tx.commit()?;
        Ok(payload.through > current_sync || divergence_changed)
    }
}

fn merge_divergence(
    tx: &Transaction<'_>,
    divergence: &SyncDivergence,
) -> Result<bool, PersonalError> {
    let records: i64 = tx.query_row(
        "SELECT count(*) FROM canonical_records WHERE digest IN (?1,?2)",
        params![divergence.digest_a, divergence.digest_b],
        |row| row.get(0),
    )?;
    if records != 2 {
        return Err(PersonalError::Conflict(
            "sync divergence references missing record".into(),
        ));
    }
    let id = crate::divergence_id(&divergence.digest_a, &divergence.digest_b);
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO personal_divergences
         (id,digest_a,digest_b,status,created_at,acknowledged_at)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![
            id,
            divergence.digest_a,
            divergence.digest_b,
            divergence.status,
            divergence.created_at,
            divergence.acknowledged_at
        ],
    )? == 1;
    let acknowledged = if divergence.status == "acknowledged" {
        tx.execute(
            "UPDATE personal_divergences SET status='acknowledged',acknowledged_at=?1
             WHERE digest_a=?2 AND digest_b=?3 AND status='unacknowledged'",
            params![
                divergence.acknowledged_at,
                divergence.digest_a,
                divergence.digest_b
            ],
        )? == 1
    } else {
        false
    };
    Ok(inserted || acknowledged)
}

fn merge_record(tx: &Transaction<'_>, record: &SyncRecord) -> Result<(), PersonalError> {
    let metadata = serde_json_canonicalizer::to_string(&record.metadata)
        .map_err(|e| PersonalError::Metadata(e.to_string()))?;
    tx.execute(
        "INSERT OR IGNORE INTO canonical_records
             (digest, identity_domain, content, occurred_at, metadata, created_at, tombstoned)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            record.content_digest,
            IDENTITY_DOMAIN,
            record.text,
            record.occurred_at,
            metadata,
            record.created_at,
            record.tombstoned as i64
        ],
    )?;
    tx.execute(
        "UPDATE canonical_records SET created_at = min(created_at, ?1),
                tombstoned = max(tombstoned, ?2)
         WHERE digest = ?3",
        params![
            record.created_at,
            record.tombstoned as i64,
            record.content_digest
        ],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO replica_records(record_digest) VALUES (?1)",
        [&record.content_digest],
    )?;
    for tag in &record.tags {
        if !tag.trim().is_empty() {
            tx.execute("INSERT OR IGNORE INTO tags(name) VALUES (?1)", [tag])?;
            tx.execute(
                "INSERT OR IGNORE INTO record_tags(record_digest, tag) VALUES (?1, ?2)",
                params![record.content_digest, tag],
            )?;
        }
    }
    for sighting in &record.sightings {
        let existing: Option<String> = tx
            .query_row(
                "SELECT record_digest FROM sightings WHERE origin_device = ?1 AND origin_id = ?2",
                params![sighting.origin_device, sighting.origin_record_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing
            .as_deref()
            .is_some_and(|digest| digest != record.content_digest)
        {
            return Err(PersonalError::Conflict(
                "replica sighting origin conflict".into(),
            ));
        }
        tx.execute(
            "INSERT OR IGNORE INTO sightings
             (origin_device, origin_id, record_digest, created_at, source, conversation)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                sighting.origin_device,
                sighting.origin_record_id,
                record.content_digest,
                sighting.captured_at,
                sighting.source,
                sighting.conversation_id
            ],
        )?;
        tx.execute(
            "UPDATE canonical_records SET created_at = min(created_at, ?1) WHERE digest = ?2",
            params![sighting.captured_at, record.content_digest],
        )?;
    }
    Ok(())
}
