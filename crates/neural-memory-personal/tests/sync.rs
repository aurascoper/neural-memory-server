use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use neural_memory_personal::*;
use serde_json::json;

const T0: &str = "2026-08-06T12:00:00.000Z";
const T1: &str = "2026-08-06T12:00:01.000Z";

fn key() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}

#[test]
fn ed25519_matches_cross_language_fixture_and_rejects_hostile_signatures() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../docs/ed25519-v1-vectors.json")).unwrap();
    assert_eq!(fixture["version"], "ed25519-v1");
    let decode = |field: &serde_json::Value| BASE64.decode(field.as_str().unwrap()).unwrap();
    let message = decode(&fixture["messageBase64"]);
    let public: [u8; 32] = decode(&fixture["publicKeyBase64"]).try_into().unwrap();
    let key = VerifyingKey::from_bytes(&public).unwrap();
    let valid = Signature::from_slice(&decode(&fixture["validSignatureBase64"])).unwrap();
    key.verify(&message, &valid).unwrap();
    let mut tampered = message.clone();
    tampered[0] ^= 1;
    assert!(key.verify(&tampered, &valid).is_err());
    for rejection in fixture["rejections"].as_array().unwrap() {
        let signature = Signature::from_slice(&decode(&rejection["signatureBase64"])).unwrap();
        assert!(
            key.verify(&message, &signature).is_err(),
            "{}",
            rejection["name"]
        );
    }
}

fn sighting(device: &str, id: &str, at: &str) -> Sighting {
    Sighting {
        created_at: at.into(),
        source: Some("test".into()),
        conversation: Some("conversation".into()),
        origin_device: device.into(),
        origin_id: id.into(),
    }
}

fn capture<'a>(text: &'a str, id: &str, at: &str) -> Capture<'a> {
    Capture {
        content: text,
        occurred_at: None,
        metadata_json: "{}",
        sighting: sighting("gpd", id, at),
        tags: vec!["personal".into()],
    }
}

fn record(text: &str, tombstoned: bool, origin_id: &str) -> SyncRecord {
    let digest = identity(text, None, "{}").unwrap().0;
    SyncRecord {
        content_domain: IDENTITY_DOMAIN.into(),
        content_digest: digest,
        text: text.into(),
        occurred_at: None,
        metadata: json!({}),
        created_at: T0.into(),
        tombstoned,
        tags: vec!["replica".into()],
        sightings: vec![SyncSighting {
            origin_device: "mac".into(),
            origin_record_id: origin_id.into(),
            captured_at: T0.into(),
            source: None,
            conversation_id: None,
        }],
    }
}

fn payload(from: SyncCursor, changes: Vec<SyncChange>) -> SyncPayloadV1 {
    let through = changes.last().map_or(from, |change| change.cursor);
    SyncPayloadV1 {
        source_device: "mac".into(),
        from_exclusive: from,
        through,
        generated_at: T1.into(),
        changes,
        divergences: vec![],
    }
}

fn change(cursor: SyncCursor, operation: &str, record: SyncRecord) -> SyncChange {
    SyncChange {
        cursor,
        operation: operation.into(),
        record,
    }
}

fn sign_exact_payload(payload: &[u8], signing: &SigningKey) -> SyncEnvelopeV1 {
    SyncEnvelopeV1 {
        version: "SyncBundleV1".into(),
        algorithm: "Ed25519".into(),
        signer_key_id: signer_key_id(&signing.verifying_key()),
        payload_base64: BASE64.encode(payload),
        signature_base64: BASE64.encode(signing.sign(payload).to_bytes()),
    }
}

#[test]
fn envelope_signs_exact_payload_and_rejects_tampering() {
    let signing = key();
    let payload = payload(
        SyncCursor {
            epoch: 0,
            sequence: 0,
        },
        vec![],
    );
    let envelope = sign_payload(&payload, &signing).unwrap();
    assert_eq!(envelope.version, "SyncBundleV1");
    assert_eq!(envelope.algorithm, "Ed25519");
    assert_eq!(
        envelope.signer_key_id,
        signer_key_id(&signing.verifying_key())
    );
    assert_eq!(
        verify_envelope(&envelope, &signing.verifying_key()).unwrap(),
        payload
    );

    let mut tampered = envelope.clone();
    let mut payload_bytes = BASE64.decode(&tampered.payload_base64).unwrap();
    payload_bytes[0] ^= 1;
    tampered.payload_base64 = BASE64.encode(payload_bytes);
    assert!(verify_envelope(&tampered, &signing.verifying_key()).is_err());
    let other = SigningKey::from_bytes(&[8; 32]);
    assert!(verify_envelope(&envelope, &other.verifying_key()).is_err());
}

#[test]
fn old_payload_without_divergences_decodes_as_empty() {
    let signing = key();
    let mut value = serde_json::to_value(payload(
        SyncCursor {
            epoch: 0,
            sequence: 0,
        },
        vec![],
    ))
    .unwrap();
    value.as_object_mut().unwrap().remove("divergences");
    let bytes = serde_json::to_vec(&value).unwrap();
    let decoded = verify_envelope(
        &sign_exact_payload(&bytes, &signing),
        &signing.verifying_key(),
    )
    .unwrap();
    assert!(decoded.divergences.is_empty());
}

#[test]
fn importer_rejects_epoch_skip() {
    let signing = key();
    let bundle = sign_payload(
        &payload(
            SyncCursor {
                epoch: 0,
                sequence: 0,
            },
            vec![change(
                SyncCursor {
                    epoch: 2,
                    sequence: 1,
                },
                "upsert",
                record("skip", false, "skip"),
            )],
        ),
        &signing,
    )
    .unwrap();
    let mut store = PersonalStore::open_in_memory().unwrap();
    assert!(store
        .import_verified_bundle(&bundle, &signing.verifying_key())
        .is_err());
}

#[test]
fn importer_rejects_mixed_old_and_successor_epoch_changes() {
    let signing = key();
    let bundle = sign_payload(
        &payload(
            SyncCursor {
                epoch: 1,
                sequence: 1,
            },
            vec![
                change(
                    SyncCursor {
                        epoch: 1,
                        sequence: 2,
                    },
                    "upsert",
                    record("old", false, "old"),
                ),
                change(
                    SyncCursor {
                        epoch: 2,
                        sequence: 1,
                    },
                    "upsert",
                    record("new", false, "new"),
                ),
            ],
        ),
        &signing,
    )
    .unwrap();
    assert!(verify_envelope(&bundle, &signing.verifying_key()).is_ok());
    let mut store = PersonalStore::open_in_memory().unwrap();
    store
        .advance_cursor(&Cursor {
            epoch: 1,
            sequence: 1,
            replicated_as_of: None,
        })
        .unwrap();
    assert!(store
        .import_verified_bundle(&bundle, &signing.verifying_key())
        .is_err());
}

#[test]
fn verified_payload_rejects_metadata_keys_before_map_construction() {
    let signing = key();
    let valid = payload(
        SyncCursor {
            epoch: 0,
            sequence: 0,
        },
        vec![change(
            SyncCursor {
                epoch: 1,
                sequence: 1,
            },
            "upsert",
            record("replicated", false, "metadata-preflight"),
        )],
    );
    let serialized = serde_json::to_string(&valid).unwrap();
    assert!(serialized.contains(r#""metadata":{}"#));

    for (metadata, category) in [
        (r#"{"e\u0301":1}"#, "non-NFC metadata key"),
        (r#"{"\u00e9":1,"e\u0301":2}"#, "metadata key NFC collision"),
        (r#"{"key":1,"\u006bey":2}"#, "duplicate metadata key"),
    ] {
        let hostile =
            serialized.replacen(r#""metadata":{}"#, &format!(r#""metadata":{metadata}"#), 1);
        let envelope = sign_exact_payload(hostile.as_bytes(), &signing);
        let error = verify_envelope(&envelope, &signing.verifying_key()).unwrap_err();
        assert!(error.to_string().contains(category), "{error}");
    }
}

#[test]
fn local_changes_export_in_order_and_wait_for_ack() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    store.capture(&capture("one", "1", T0)).unwrap();
    store.capture(&capture("two", "2", T1)).unwrap();
    store.capture(&capture("one", "1", T0)).unwrap();

    let exported = store
        .export_promotions(
            "gpd",
            SyncCursor {
                epoch: 0,
                sequence: 0,
            },
            T1,
        )
        .unwrap();
    assert_eq!(exported.changes.len(), 2, "replay allocates no change");
    assert_eq!(exported.changes[0].cursor.sequence, 1);
    assert_eq!(exported.changes[1].cursor.sequence, 2);
    assert_eq!(exported.through.sequence, 2);

    let pending = || -> i64 {
        store
            .conn
            .query_row(
                "SELECT count(*) FROM promotion_outbox WHERE status = 'pending'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(pending(), 2, "export must not acknowledge Mac commit");
    assert_eq!(
        store
            .acknowledge_promotions(
                SyncCursor {
                    epoch: 1,
                    sequence: 1,
                },
                T1,
            )
            .unwrap(),
        1
    );
    assert_eq!(pending(), 1);
}

#[test]
fn verified_bundle_replay_is_a_no_op() {
    let signing = key();
    let bundle = sign_payload(
        &payload(
            SyncCursor {
                epoch: 0,
                sequence: 0,
            },
            vec![change(
                SyncCursor {
                    epoch: 1,
                    sequence: 1,
                },
                "upsert",
                record("replicated", false, "1"),
            )],
        ),
        &signing,
    )
    .unwrap();
    let mut store = PersonalStore::open_in_memory().unwrap();
    assert!(store
        .import_verified_bundle(&bundle, &signing.verifying_key())
        .unwrap());
    assert!(!store
        .import_verified_bundle(&bundle, &signing.verifying_key())
        .unwrap());
    let sightings: i64 = store
        .conn
        .query_row("SELECT count(*) FROM sightings", [], |row| row.get(0))
        .unwrap();
    assert_eq!(sightings, 1);
}

#[test]
fn import_rejects_cursor_gap_without_writes() {
    let signing = key();
    let bundle = sign_payload(
        &payload(
            SyncCursor {
                epoch: 0,
                sequence: 0,
            },
            vec![change(
                SyncCursor {
                    epoch: 1,
                    sequence: 2,
                },
                "upsert",
                record("gap", false, "gap"),
            )],
        ),
        &signing,
    )
    .unwrap();
    let mut store = PersonalStore::open_in_memory().unwrap();
    assert!(store
        .import_verified_bundle(&bundle, &signing.verifying_key())
        .is_err());
    assert_eq!(store.cursor().unwrap().sequence, 0);
}

#[test]
fn bad_digest_rolls_back_the_whole_bundle() {
    let signing = key();
    let mut bad = record("bad", false, "bad");
    bad.content_digest = "0".repeat(64);
    let bundle = sign_payload(
        &payload(
            SyncCursor {
                epoch: 0,
                sequence: 0,
            },
            vec![
                change(
                    SyncCursor {
                        epoch: 1,
                        sequence: 1,
                    },
                    "upsert",
                    record("good", false, "good"),
                ),
                change(
                    SyncCursor {
                        epoch: 1,
                        sequence: 2,
                    },
                    "upsert",
                    bad,
                ),
            ],
        ),
        &signing,
    )
    .unwrap();
    let mut store = PersonalStore::open_in_memory().unwrap();
    assert!(store
        .import_verified_bundle(&bundle, &signing.verifying_key())
        .is_err());
    let records: i64 = store
        .conn
        .query_row("SELECT count(*) FROM canonical_records", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(records, 0);
    assert_eq!(store.cursor().unwrap().sequence, 0);
}

#[test]
fn later_upsert_never_resurrects_a_tombstone() {
    let signing = key();
    let first = sign_payload(
        &payload(
            SyncCursor {
                epoch: 0,
                sequence: 0,
            },
            vec![change(
                SyncCursor {
                    epoch: 1,
                    sequence: 1,
                },
                "tombstone",
                record("gone", true, "dead"),
            )],
        ),
        &signing,
    )
    .unwrap();
    let second = sign_payload(
        &payload(
            SyncCursor {
                epoch: 1,
                sequence: 1,
            },
            vec![change(
                SyncCursor {
                    epoch: 1,
                    sequence: 2,
                },
                "upsert",
                record("gone", false, "still-dead"),
            )],
        ),
        &signing,
    )
    .unwrap();
    let mut store = PersonalStore::open_in_memory().unwrap();
    store
        .import_verified_bundle(&first, &signing.verifying_key())
        .unwrap();
    store
        .import_verified_bundle(&second, &signing.verifying_key())
        .unwrap();
    let tombstoned: bool = store
        .conn
        .query_row("SELECT tombstoned != 0 FROM canonical_records", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(tombstoned);
}

#[test]
fn full_divergence_state_propagates_at_same_cursor_and_ack_is_monotonic() {
    let signing = key();
    let mut source = PersonalStore::open_in_memory().unwrap();
    let a = source.capture(&capture("tea preferred", "a", T0)).unwrap();
    let b = source.capture(&capture("tea avoided", "b", T1)).unwrap();
    let divergence = source.flag_divergence(&a, &b, T1).unwrap();
    let initial = source
        .export_promotions(
            "gpd",
            SyncCursor {
                epoch: 0,
                sequence: 0,
            },
            T1,
        )
        .unwrap();
    assert_eq!(initial.divergences.len(), 1);
    let old = sign_payload(&initial, &signing).unwrap();
    let mut replica = PersonalStore::open_in_memory().unwrap();
    assert!(replica
        .import_verified_bundle(&old, &signing.verifying_key())
        .unwrap());
    assert_eq!(replica.list_unacknowledged_divergences().unwrap().len(), 1);

    source.acknowledge_divergence(&divergence.id, T1).unwrap();
    let acknowledged = source
        .export_promotions("gpd", initial.through, T1)
        .unwrap();
    assert!(acknowledged.changes.is_empty());
    assert_eq!(acknowledged.through, initial.through);
    assert_eq!(acknowledged.divergences[0].status, "acknowledged");
    assert!(replica
        .import_verified_bundle(
            &sign_payload(&acknowledged, &signing).unwrap(),
            &signing.verifying_key(),
        )
        .unwrap());
    assert!(replica
        .list_unacknowledged_divergences()
        .unwrap()
        .is_empty());
    assert!(!replica
        .import_verified_bundle(&old, &signing.verifying_key())
        .unwrap());
    assert!(replica
        .list_unacknowledged_divergences()
        .unwrap()
        .is_empty());
}

#[test]
fn same_cursor_changed_record_transition_is_rejected_as_equivocation() {
    let signing = key();
    let from = SyncCursor {
        epoch: 0,
        sequence: 0,
    };
    let cursor = SyncCursor {
        epoch: 1,
        sequence: 1,
    };
    let accepted = payload(
        from,
        vec![change(cursor, "upsert", record("first", false, "a"))],
    );
    let mut changed_source = accepted.clone();
    changed_source.source_device = "other-mac".into();
    let mut changed_from = accepted.clone();
    changed_from.from_exclusive.sequence = 9;
    let mut changed_record = accepted.clone();
    changed_record.changes[0].record = record("different", false, "b");
    let mut changed_display_time = accepted.clone();
    changed_display_time.generated_at = T0.into();

    let mut store = PersonalStore::open_in_memory().unwrap();
    store
        .import_verified_bundle(
            &sign_payload(&accepted, &signing).unwrap(),
            &signing.verifying_key(),
        )
        .unwrap();
    assert!(store
        .import_verified_bundle(
            &sign_payload(&accepted, &signing).unwrap(),
            &signing.verifying_key(),
        )
        .is_ok_and(|changed| !changed));
    assert!(store
        .import_verified_bundle(
            &sign_payload(&changed_display_time, &signing).unwrap(),
            &signing.verifying_key(),
        )
        .is_ok_and(|changed| !changed));
    assert!(store
        .import_verified_bundle(
            &sign_payload(&changed_source, &signing).unwrap(),
            &signing.verifying_key(),
        )
        .is_err());
    assert!(store
        .import_verified_bundle(
            &sign_payload(&changed_from, &signing).unwrap(),
            &signing.verifying_key(),
        )
        .is_err());
    assert!(store
        .import_verified_bundle(
            &sign_payload(&changed_record, &signing).unwrap(),
            &signing.verifying_key(),
        )
        .is_err());
}

#[test]
fn missing_divergence_record_rolls_back_records_cursor_and_divergence() {
    let signing = key();
    let good = record("present", false, "present");
    let missing = identity("missing", None, "{}").unwrap().0;
    let bundle = sign_payload(
        &SyncPayloadV1 {
            divergences: vec![SyncDivergence {
                digest_a: good.content_digest.clone().min(missing.clone()),
                digest_b: good.content_digest.clone().max(missing),
                status: "unacknowledged".into(),
                created_at: T1.into(),
                acknowledged_at: None,
            }],
            ..payload(
                SyncCursor {
                    epoch: 0,
                    sequence: 0,
                },
                vec![change(
                    SyncCursor {
                        epoch: 1,
                        sequence: 1,
                    },
                    "upsert",
                    good,
                )],
            )
        },
        &signing,
    )
    .unwrap();
    let mut store = PersonalStore::open_in_memory().unwrap();
    assert!(store
        .import_verified_bundle(&bundle, &signing.verifying_key())
        .is_err());
    let records: i64 = store
        .conn
        .query_row("SELECT count(*) FROM canonical_records", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(records, 0);
    assert_eq!(store.cursor().unwrap().sequence, 0);
    let fingerprint: Option<String> = store
        .conn
        .query_row(
            "SELECT transition_fingerprint FROM replica_cursor WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(fingerprint, None);
    assert!(store.list_unacknowledged_divergences().unwrap().is_empty());
}
