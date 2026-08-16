use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};

use neural_memory_personal::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityVectors {
    version: String,
    content_domain: String,
    vectors: Vec<IdentityVector>,
    rejections: Vec<IdentityRejection>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityVector {
    name: String,
    #[serde(rename = "textNFC")]
    text_nfc: String,
    #[serde(rename = "equivalentTextNFD")]
    equivalent_text_nfd: String,
    occurred_at: String,
    #[serde(rename = "metadataJSON")]
    metadata_json: String,
    #[serde(rename = "canonicalUTF8")]
    canonical_utf8: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityRejection {
    name: String,
    #[serde(rename = "metadataJSON")]
    metadata_json: String,
    category: String,
}

fn fixture_rejection_category(error: &PersonalError) -> &'static str {
    match error {
        PersonalError::Metadata(message) if message.starts_with("duplicate metadata key:") => {
            "duplicate-metadata-key"
        }
        PersonalError::Metadata(message) if message.starts_with("metadata key NFC collision:") => {
            "metadata-key-nfc-collision"
        }
        PersonalError::Metadata(message) if message.starts_with("non-NFC metadata key:") => {
            "non-nfc-metadata-key"
        }
        PersonalError::Metadata(message) if message.contains("number out of range") => {
            "non-finite-number"
        }
        PersonalError::Metadata(message)
            if message.contains("surrogate") || message.contains("hex escape") =>
        {
            "invalid-unicode-scalar"
        }
        PersonalError::Metadata(_) => "invalid-json",
        PersonalError::Conflict(_) => "identity-conflict",
        PersonalError::Sql(_) => "storage-error",
    }
}

fn validate_vector(vector: &IdentityVector) -> Result<(), String> {
    let occurred_at = Some(vector.occurred_at.as_str());
    let bytes = identity_bytes(
        &vector.equivalent_text_nfd,
        occurred_at,
        &vector.metadata_json,
    )
    .map_err(|error| error.to_string())?;
    if String::from_utf8(bytes).map_err(|error| error.to_string())? != vector.canonical_utf8 {
        return Err(format!("{} canonicalUTF8 mismatch", vector.name));
    }
    let nfd = identity(
        &vector.equivalent_text_nfd,
        occurred_at,
        &vector.metadata_json,
    )
    .map_err(|error| error.to_string())?
    .0;
    let nfc = identity(&vector.text_nfc, occurred_at, &vector.metadata_json)
        .map_err(|error| error.to_string())?
        .0;
    if nfd != vector.sha256 {
        return Err(format!("{} SHA-256 mismatch: measured {nfd}", vector.name));
    }
    if nfd != nfc {
        return Err(format!("{} NFC/NFD mismatch", vector.name));
    }
    Ok(())
}

fn sighting(device: &str, id: &str, created_at: &str) -> Sighting {
    Sighting {
        created_at: created_at.into(),
        source: Some("claude-code".into()),
        conversation: Some("conversation-1".into()),
        origin_device: device.into(),
        origin_id: id.into(),
    }
}

fn capture<'a>(
    content: &'a str,
    metadata_json: &'a str,
    sighting: Sighting,
    tags: &[&str],
) -> Capture<'a> {
    Capture {
        content,
        occurred_at: None,
        metadata_json,
        sighting,
        tags: tags.iter().map(|tag| (*tag).into()).collect(),
    }
}

#[test]
fn nfc_and_nfd_text_have_one_identity() {
    let nfc = identity("caf\u{e9}", None, "{}").unwrap().0;
    let nfd = identity("cafe\u{301}", None, "{}").unwrap().0;
    assert_eq!(nfc, nfd);
    assert_ne!(nfc, identity("cafe", None, "{}").unwrap().0);
}

#[test]
fn identity_matches_the_cross_language_golden_vector() {
    let fixture: IdentityVectors =
        serde_json::from_str(include_str!("../../../docs/identity-v1-vectors.json")).unwrap();
    assert_eq!(fixture.version, "identity-v1");
    assert_eq!(fixture.content_domain, IDENTITY_DOMAIN);
    for vector in &fixture.vectors {
        validate_vector(vector).unwrap();
    }
    for rejection in fixture.rejections {
        let error = identity("fixture rejection", None, &rejection.metadata_json)
            .expect_err(&rejection.name);
        assert_eq!(
            fixture_rejection_category(&error),
            rejection.category,
            "{}: {error}",
            rejection.name
        );
    }

    let mut mutated = fixture.vectors[0].clone();
    mutated.sha256.replace_range(
        ..1,
        if &mutated.sha256[..1] == "0" {
            "1"
        } else {
            "0"
        },
    );
    assert!(validate_vector(&mutated).is_err());
}

#[test]
fn metadata_uses_jcs_key_and_number_canonicalization() {
    let a = identity("same", None, r#"{"z":1e30,"a":{"b":0.000001,"a":1.0}}"#).unwrap();
    let b = identity("same", None, r#"{"a":{"a":1,"b":1e-6},"z":1e+30}"#).unwrap();
    assert_eq!(
        a.0, b.0,
        "hostile key order and number spelling must converge"
    );
    assert_eq!(a.2, r#"{"a":{"a":1,"b":0.000001},"z":1e+30}"#);
    assert!(identity("same", None, r#"{"bad":NaN}"#).is_err());
    assert!(identity("same", None, r#"{"bad":1e999}"#).is_err());
}

#[test]
fn occurred_at_is_identity_but_undated_content_converges() {
    let undated_a = identity("same", None, "{}").unwrap().0;
    let undated_b = identity("same", None, "{}").unwrap().0;
    let dated = identity("same", Some("2026-08-06T00:00:00.000Z"), "{}")
        .unwrap()
        .0;
    assert_eq!(undated_a, undated_b);
    assert_ne!(undated_a, dated);
}

#[test]
fn occurred_at_rejects_noncanonical_and_invalid_spellings() {
    for invalid in [
        "2026-08-06T12:34:56Z",
        "2026-08-06T12:34:56.78Z",
        "2026-08-06T12:34:56.7890Z",
        "2026-08-06T12:34:56.789+00:00",
        "2026-08-06t12:34:56.789z",
        "2026-02-29T12:34:56.789Z",
        "2024-02-30T12:34:56.789Z",
        "2026-08-06T24:00:00.000Z",
        "2026-08-06T12:60:00.000Z",
        "2026-08-06T12:34:60.000Z",
    ] {
        assert!(identity("same", Some(invalid), "{}").is_err(), "{invalid}");
    }
    assert!(identity("same", Some("2024-02-29T23:59:59.999Z"), "{}").is_ok());
}

#[test]
fn replay_is_idempotent_and_origin_reuse_cannot_repoint() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    let item = capture(
        "remember this",
        "{}",
        sighting("gpd", "1", "2026-08-06T02:00:00.000Z"),
        &["one"],
    );
    let digest = store.capture(&item).unwrap();
    assert_eq!(digest, store.capture(&item).unwrap());
    let record = store.get(&digest).unwrap().unwrap();
    assert_eq!(record.sighting_count, 1);
    let pending: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM promotion_outbox
             WHERE record_digest = ?1 AND status = 'pending'",
            [&digest],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1, "capture and outbox enqueue are idempotent");

    let conflict = capture(
        "different",
        "{}",
        sighting("gpd", "1", "2026-08-06T02:00:00.000Z"),
        &[],
    );
    assert!(matches!(
        store.capture(&conflict),
        Err(PersonalError::Conflict(_))
    ));
}

#[test]
fn digest_match_merges_sightings_min_created_at_and_tag_union() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    let later = capture(
        "shared",
        r#"{"kind":"preference"}"#,
        sighting("gpd", "local-1", "2026-08-06T12:00:00.000Z"),
        &["voice", "local"],
    );
    let digest = store.capture(&later).unwrap();
    let earlier = capture(
        "shared",
        r#"{"kind":"preference"}"#,
        sighting("mac", "remote-9", "2026-08-05T12:00:00.000Z"),
        &["voice", "replicated"],
    );
    assert_eq!(digest, store.merge_replica(&earlier).unwrap());

    let record = store.get(&digest).unwrap().unwrap();
    assert_eq!(record.created_at, "2026-08-05T12:00:00.000Z");
    assert_eq!(record.sighting_count, 2);
    assert_eq!(record.tags, vec!["local", "replicated", "voice"]);
    let captures: i64 = store
        .conn
        .query_row("SELECT count(*) FROM captures", [], |row| row.get(0))
        .unwrap();
    assert_eq!(captures, 1, "a replica sighting is not a local capture");
    let replicas: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM replica_records WHERE record_digest = ?1",
            [&digest],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(replicas, 1);
    let outbox: i64 = store
        .conn
        .query_row("SELECT count(*) FROM promotion_outbox", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(outbox, 1, "replica merge must not enqueue promotion");
}

#[test]
fn replica_only_record_has_membership_but_no_capture_or_outbox() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    let mut remote_sighting = sighting("mac", "remote-only", "2026-08-06T12:00:00.000Z");
    remote_sighting.source = None;
    let remote = capture("remote", "{}", remote_sighting, &[]);
    let digest = store.merge_replica(&remote).unwrap();
    for table in ["captures", "promotion_outbox"] {
        let count: i64 = store
            .conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "replica must not enter {table}");
    }
    let membership: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM replica_records WHERE record_digest = ?1",
            [&digest],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(membership, 1);
    let source_is_null: bool = store
        .conn
        .query_row(
            "SELECT source IS NULL FROM sightings WHERE record_digest = ?1",
            [&digest],
            |row| row.get(0),
        )
        .unwrap();
    assert!(source_is_null);
}

#[test]
fn cursor_authority_is_epoch_then_sequence_only() {
    let store = PersonalStore::open_in_memory().unwrap();
    assert!(store
        .advance_cursor(&Cursor {
            epoch: 2,
            sequence: 7,
            replicated_as_of: Some("later display".into()),
        })
        .unwrap());
    assert!(!store
        .advance_cursor(&Cursor {
            epoch: 2,
            sequence: 7,
            replicated_as_of: Some("different display".into()),
        })
        .unwrap());
    assert!(!store
        .advance_cursor(&Cursor {
            epoch: 1,
            sequence: 999,
            replicated_as_of: Some("newer-looking display".into()),
        })
        .unwrap());
    assert!(store
        .advance_cursor(&Cursor {
            epoch: 3,
            sequence: 0,
            replicated_as_of: Some("older-looking display".into()),
        })
        .unwrap());
    assert_eq!(
        store.cursor().unwrap(),
        Cursor {
            epoch: 3,
            sequence: 0,
            replicated_as_of: Some("older-looking display".into()),
        }
    );
}

#[test]
fn divergence_is_unordered_idempotent_and_acknowledged_without_lww() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    let a = store
        .capture(&capture(
            "Tea is preferred",
            "{}",
            sighting("gpd", "div-a", "2026-08-06T10:00:00.000Z"),
            &[],
        ))
        .unwrap();
    let b = store
        .capture(&capture(
            "Tea is avoided",
            "{}",
            sighting("gpd", "div-b", "2026-08-06T10:01:00.000Z"),
            &[],
        ))
        .unwrap();

    let first = store
        .flag_divergence(&a, &b, "2026-08-06T10:02:00.000Z")
        .unwrap();
    let replay = store
        .flag_divergence(&b, &a, "2026-08-06T10:03:00.000Z")
        .unwrap();
    assert_eq!(first, replay);
    assert!(first.digest_a < first.digest_b);
    assert_eq!(first.status, "unacknowledged");
    let listed = store.list_unacknowledged_divergences().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], first);

    for digest in [&a, &b] {
        let (content, tombstoned): (String, bool) = store
            .conn
            .query_row(
                "SELECT content,tombstoned != 0 FROM canonical_records WHERE digest=?1",
                [digest],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(!tombstoned);
        assert!(content == "Tea is preferred" || content == "Tea is avoided");
    }

    assert!(store
        .acknowledge_divergence(&first.id, "2026-08-06T10:04:00.000Z")
        .unwrap());
    assert!(!store
        .acknowledge_divergence(&first.id, "2026-08-06T10:05:00.000Z")
        .unwrap());
    assert!(store.list_unacknowledged_divergences().unwrap().is_empty());
    assert!(store.get(&a).unwrap().is_some());
    assert!(store.get(&b).unwrap().is_some());
}

#[test]
fn opening_an_existing_v3_database_applies_later_personal_migrations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("personal.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE personal_schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);",
        )
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0001_init.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0002_embeddings.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0003_embedding_failures.sql"))
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO personal_schema_migrations(version,applied_at)
             VALUES (1,'v1'),(2,'v2'),(3,'v3');",
        )
        .unwrap();
    drop(connection);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    let store = PersonalStore::open(&path).unwrap();
    let migrations: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM personal_schema_migrations WHERE version IN (4,5)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let table: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='personal_divergences'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let fingerprint_column: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('replica_cursor')
             WHERE name='transition_fingerprint'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!((migrations, table, fingerprint_column), (2, 1, 1));
}

#[test]
fn reenrollment_reset_removes_only_replica_only_state() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    let local = store
        .capture(&capture(
            "local survives",
            "{}",
            sighting("gpd", "local-reset", "2026-08-06T11:00:00.000Z"),
            &[],
        ))
        .unwrap();
    let replica = store
        .merge_replica(&capture(
            "replica removed",
            "{}",
            sighting("mac", "remote-reset", "2026-08-06T11:01:00.000Z"),
            &[],
        ))
        .unwrap();
    store
        .advance_cursor(&Cursor {
            epoch: 3,
            sequence: 7,
            replicated_as_of: Some("display".into()),
        })
        .unwrap();
    store
        .conn
        .execute(
            "UPDATE replica_cursor SET transition_fingerprint=?1 WHERE singleton=1",
            ["a".repeat(64)],
        )
        .unwrap();

    assert!(store.reset_replica_for_reenrollment(2).is_err());
    assert_eq!(store.reset_replica_for_reenrollment(3).unwrap(), 1);
    assert!(store.get(&local).unwrap().is_some());
    assert!(store.get(&replica).unwrap().is_none());
    assert_eq!(
        (
            store.cursor().unwrap().epoch,
            store.cursor().unwrap().sequence
        ),
        (3, 0)
    );
    let captures: i64 = store
        .conn
        .query_row("SELECT count(*) FROM captures", [], |row| row.get(0))
        .unwrap();
    let promotions: i64 = store
        .conn
        .query_row("SELECT count(*) FROM promotion_changes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!((captures, promotions), (1, 1));
    let fingerprint: Option<String> = store
        .conn
        .query_row(
            "SELECT transition_fingerprint FROM replica_cursor WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fingerprint, None);
}

#[test]
fn promotion_epoch_bump_allocates_complete_local_snapshot_from_sequence_one() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    store
        .capture(&capture(
            "one",
            "{}",
            sighting("gpd", "snap-1", "2026-08-06T11:00:00.000Z"),
            &[],
        ))
        .unwrap();
    store
        .capture(&capture(
            "two",
            "{}",
            sighting("gpd", "snap-2", "2026-08-06T11:01:00.000Z"),
            &[],
        ))
        .unwrap();
    let through = store
        .bump_promotion_epoch_and_snapshot("2026-08-06T11:02:00.000Z")
        .unwrap();
    assert_eq!((through.epoch, through.sequence), (2, 2));
    let payload = store
        .export_promotions(
            "gpd",
            SyncCursor {
                epoch: 1,
                sequence: 2,
            },
            "2026-08-06T11:02:00.000Z",
        )
        .unwrap();
    assert_eq!(payload.changes.len(), 2);
    assert_eq!(
        payload.changes[0].cursor,
        SyncCursor {
            epoch: 2,
            sequence: 1
        }
    );
    assert_eq!(
        payload.changes[1].cursor,
        SyncCursor {
            epoch: 2,
            sequence: 2
        }
    );
}

#[test]
fn export_stops_at_epoch_boundary_and_divergences_only_ship_at_head() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    let a = store
        .capture(&capture(
            "old-a",
            "{}",
            sighting("gpd", "chunk-a", "2026-08-06T11:00:00.000Z"),
            &[],
        ))
        .unwrap();
    let b = store
        .capture(&capture(
            "old-b",
            "{}",
            sighting("gpd", "chunk-b", "2026-08-06T11:01:00.000Z"),
            &[],
        ))
        .unwrap();
    store
        .flag_divergence(&a, &b, "2026-08-06T11:01:00.000Z")
        .unwrap();
    store
        .bump_promotion_epoch_and_snapshot("2026-08-06T11:02:00.000Z")
        .unwrap();

    let old_tail = store
        .export_promotions(
            "gpd",
            SyncCursor {
                epoch: 1,
                sequence: 0,
            },
            "2026-08-06T11:03:00.000Z",
        )
        .unwrap();
    assert_eq!(
        old_tail.through,
        SyncCursor {
            epoch: 1,
            sequence: 2
        }
    );
    assert!(old_tail
        .changes
        .iter()
        .all(|change| change.cursor.epoch == 1));
    assert!(old_tail.divergences.is_empty());

    let snapshot = store
        .export_promotions("gpd", old_tail.through, "2026-08-06T11:03:00.000Z")
        .unwrap();
    assert_eq!(
        snapshot.changes[0].cursor,
        SyncCursor {
            epoch: 2,
            sequence: 1
        }
    );
    assert_eq!(
        snapshot.through,
        SyncCursor {
            epoch: 2,
            sequence: 2
        }
    );
    assert_eq!(snapshot.divergences.len(), 1);
}

#[test]
fn personal_schema_lives_in_a_different_file() {
    let dir = tempfile::tempdir().unwrap();
    let personal_path = dir.path().join("personal.db");
    let store = PersonalStore::open(&personal_path).unwrap();
    let evidence_tables: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('memories', 'observations')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(evidence_tables, 0);
    assert!(personal_path.exists());
}

#[test]
fn personal_database_is_created_private_and_insecure_existing_files_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("personal.db");
    drop(PersonalStore::open(&path).unwrap());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    assert!(PersonalStore::open(&path).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    let link = dir.path().join("linked.db");
    symlink(&path, &link).unwrap();
    assert!(PersonalStore::open(&link).is_err());
}
