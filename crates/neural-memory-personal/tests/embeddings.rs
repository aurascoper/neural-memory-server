use ed25519_dalek::SigningKey;
use neural_memory_personal::embeddings::{
    DeterministicTestEmbedder, EmbeddingProfile, LlamaCppEmbedder, PersonalEmbedder,
};
use neural_memory_personal::*;
use serde_json::json;
use std::cell::Cell;

const T0: &str = "2026-08-06T12:00:00.000Z";
const T1: &str = "2026-08-06T12:00:01.000Z";

fn capture<'a>(text: &'a str, id: &str) -> Capture<'a> {
    Capture {
        content: text,
        occurred_at: None,
        metadata_json: "{}",
        sighting: Sighting {
            created_at: T0.into(),
            source: Some("test".into()),
            conversation: None,
            origin_device: "gpd".into(),
            origin_id: id.into(),
        },
        tags: vec!["private".into()],
    }
}

struct RejectFirstEmbedder {
    profile: EmbeddingProfile,
    calls: Cell<usize>,
}

impl RejectFirstEmbedder {
    fn new() -> Self {
        Self {
            profile: DeterministicTestEmbedder::new(8).profile().clone(),
            calls: Cell::new(0),
        }
    }
}

impl PersonalEmbedder for RejectFirstEmbedder {
    fn profile(&self) -> &EmbeddingProfile {
        &self.profile
    }

    fn probe(&self) -> Result<(), String> {
        Ok(())
    }

    fn embed_document(&self, _text: &str) -> Result<Vec<f32>, String> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        if call == 0 {
            Err("record-rejected:input-too-large".into())
        } else {
            DeterministicTestEmbedder::new(8).embed_document("later")
        }
    }
}

#[test]
fn over_limit_record_is_stale_and_does_not_starve_later_records() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    store.capture(&capture("first", "1")).unwrap();
    store.capture(&capture("later", "2")).unwrap();
    let embedder = RejectFirstEmbedder::new();
    store.set_embedding_profile(embedder.profile(), T0).unwrap();

    assert_eq!(store.rebuild_embeddings(&embedder, 10, T1).unwrap(), 1);
    let status = store
        .embedding_status(std::path::Path::new("personal.db"))
        .unwrap();
    assert_eq!((status.pending, status.ready, status.stale), (0, 1, 1));
    let failure: (String, String) = store
        .conn
        .query_row(
            "SELECT reason,failed_at FROM personal_embedding_failures",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(failure, ("input-too-large".into(), T1.into()));
}

#[test]
fn deterministic_vectors_are_stable_and_profile_mismatch_isolated() {
    let embedder = DeterministicTestEmbedder::new(8);
    let identity = embedder.profile().identity().unwrap();
    let variants = [
        EmbeddingProfile {
            backend: "other-backend".into(),
            ..embedder.profile().clone()
        },
        EmbeddingProfile {
            model_artifact: "other-artifact".into(),
            ..embedder.profile().clone()
        },
        EmbeddingProfile {
            dimension: 9,
            ..embedder.profile().clone()
        },
        EmbeddingProfile {
            normalization: "none".into(),
            ..embedder.profile().clone()
        },
        EmbeddingProfile {
            version: "test-v2".into(),
            ..embedder.profile().clone()
        },
        EmbeddingProfile {
            adapter: "llama-cpp-http".into(),
            ..embedder.profile().clone()
        },
        EmbeddingProfile {
            endpoint: Some("http://127.0.0.1:8082".into()),
            ..embedder.profile().clone()
        },
    ];
    for variant in variants {
        assert_ne!(identity, variant.identity().unwrap());
    }
    assert_eq!(
        embedder.embed_document("same").unwrap(),
        embedder.embed_document("same").unwrap()
    );
    assert_ne!(
        embedder.embed_document("same").unwrap(),
        embedder.embed_document("different").unwrap()
    );

    let mut store = PersonalStore::open_in_memory().unwrap();
    store.capture(&capture("memory", "1")).unwrap();
    store.set_embedding_profile(embedder.profile(), T0).unwrap();
    let wrong = DeterministicTestEmbedder::new(16);
    assert!(store.rebuild_embeddings(&wrong, 10, T1).is_err());
    assert_eq!(store.rebuild_embeddings(&embedder, 10, T1).unwrap(), 1);
}

#[test]
fn activation_does_not_conflate_endpoint_or_adapter_configuration() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    let first = EmbeddingProfile {
        backend: "llama.cpp-vulkan".into(),
        model_artifact: "sha256:model".into(),
        dimension: 8,
        normalization: "l2".into(),
        version: "v1".into(),
        adapter: "llama-cpp-http".into(),
        endpoint: Some("http://127.0.0.1:8082".into()),
    };
    let second = EmbeddingProfile {
        endpoint: Some("http://127.0.0.1:8083".into()),
        ..first.clone()
    };
    let first_id = store.set_embedding_profile(&first, T0).unwrap();
    let second_id = store.set_embedding_profile(&second, T1).unwrap();
    assert_ne!(first_id, second_id);
    let profiles: i64 = store
        .conn
        .query_row(
            "SELECT count(*) FROM personal_embedding_profiles",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(profiles, 2);
    assert_eq!(
        store
            .embedding_status(std::path::Path::new("personal.db"))
            .unwrap()
            .profile_identity,
        Some(second_id)
    );
}

#[test]
fn profile_rotation_invalidates_and_rebuilds_without_mixing_spaces() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    store.capture(&capture("rotate me", "1")).unwrap();
    let first = DeterministicTestEmbedder::new(8);
    let first_id = store.set_embedding_profile(first.profile(), T0).unwrap();
    store.rebuild_embeddings(&first, 10, T1).unwrap();
    let second = DeterministicTestEmbedder::new(16);
    let second_id = store.set_embedding_profile(second.profile(), T1).unwrap();
    assert_ne!(first_id, second_id);
    let status = store
        .embedding_status(std::path::Path::new("personal.db"))
        .unwrap();
    assert_eq!((status.ready, status.pending, status.stale), (0, 1, 1));
    store.rebuild_embeddings(&second, 10, T1).unwrap();
    let status = store
        .embedding_status(std::path::Path::new("personal.db"))
        .unwrap();
    assert_eq!((status.ready, status.pending, status.stale), (1, 0, 1));
    let profiles: i64 = store
        .conn
        .query_row(
            "SELECT count(DISTINCT profile_identity) FROM personal_embeddings",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        profiles, 2,
        "profiles remain separate rather than being mixed"
    );
}

#[test]
fn reapplying_active_profile_does_not_requeue_ready_records() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    store.capture(&capture("ready record", "1")).unwrap();
    let embedder = DeterministicTestEmbedder::new(8);
    let identity = store.set_embedding_profile(embedder.profile(), T0).unwrap();
    assert_eq!(store.rebuild_embeddings(&embedder, 10, T1).unwrap(), 1);
    assert_eq!(
        store
            .conn
            .query_row("SELECT count(*) FROM personal_embedding_queue", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );

    assert_eq!(
        store.set_embedding_profile(embedder.profile(), T1).unwrap(),
        identity
    );
    assert_eq!(
        store
            .conn
            .query_row("SELECT count(*) FROM personal_embedding_queue", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0,
        "identical activation must preserve ready records"
    );
}

#[test]
fn import_queues_local_derivation_and_wire_has_no_vector_fields() {
    let signing = SigningKey::from_bytes(&[31; 32]);
    let digest = identity("Mac memory", None, "{}").unwrap().0;
    let cursor = SyncCursor {
        epoch: 1,
        sequence: 1,
    };
    let payload = SyncPayloadV1 {
        source_device: "mac".into(),
        from_exclusive: SyncCursor {
            epoch: 0,
            sequence: 0,
        },
        through: cursor,
        generated_at: T1.into(),
        changes: vec![SyncChange {
            cursor,
            operation: "upsert".into(),
            record: SyncRecord {
                content_domain: IDENTITY_DOMAIN.into(),
                content_digest: digest,
                text: "Mac memory".into(),
                occurred_at: None,
                metadata: json!({}),
                created_at: T0.into(),
                tombstoned: false,
                tags: vec![],
                sightings: vec![SyncSighting {
                    origin_device: "mac".into(),
                    origin_record_id: "1".into(),
                    captured_at: T0.into(),
                    source: None,
                    conversation_id: None,
                }],
            },
        }],
        divergences: vec![],
    };
    let wire = serde_json::to_string(&payload).unwrap();
    for forbidden in ["embedding", "vector", "profileIdentity"] {
        assert!(!wire.contains(forbidden));
    }
    let envelope = sign_payload(&payload, &signing).unwrap();
    let mut store = PersonalStore::open_in_memory().unwrap();
    let embedder = DeterministicTestEmbedder::new(8);
    store.set_embedding_profile(embedder.profile(), T0).unwrap();
    store
        .import_verified_bundle(&envelope, &signing.verifying_key())
        .unwrap();
    let pending: i64 = store
        .conn
        .query_row("SELECT count(*) FROM personal_embedding_queue", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(pending, 1);
    assert_eq!(store.rebuild_embeddings(&embedder, 10, T1).unwrap(), 1);
}

#[test]
fn tombstones_are_excluded_from_queue_vectors_and_context() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    let digest = store.capture(&capture("do not return", "1")).unwrap();
    let embedder = DeterministicTestEmbedder::new(8);
    store.set_embedding_profile(embedder.profile(), T0).unwrap();
    store.rebuild_embeddings(&embedder, 10, T1).unwrap();
    store.forget(&digest, T1).unwrap();
    for table in ["personal_embedding_queue", "personal_embeddings"] {
        let count: i64 = store
            .conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
    assert!(store.local_context("return", 10).unwrap()["records"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn unavailable_production_adapter_fails_closed() {
    let profile = EmbeddingProfile {
        backend: "llama.cpp-vulkan".into(),
        model_artifact: "sha256:deadbeef".into(),
        dimension: 8,
        normalization: "l2".into(),
        version: "v1".into(),
        adapter: "llama-cpp-http".into(),
        endpoint: Some("http://127.0.0.1:9".into()),
    };
    let adapter = LlamaCppEmbedder::new(profile).unwrap();
    assert!(adapter.probe().is_err());
    let remote = EmbeddingProfile {
        endpoint: Some("https://example.com".into()),
        ..adapter.profile().clone()
    };
    assert!(LlamaCppEmbedder::new(remote).is_err());
}

#[test]
fn production_endpoint_parser_rejects_hostile_and_ambiguous_urls() {
    let base = EmbeddingProfile {
        backend: "llama.cpp-vulkan".into(),
        model_artifact: "sha256:model".into(),
        dimension: 8,
        normalization: "l2".into(),
        version: "v1".into(),
        adapter: "llama-cpp-http".into(),
        endpoint: None,
    };
    for valid in [
        "http://127.0.0.1:8082",
        "http://localhost:8082",
        "http://[::1]:8082",
    ] {
        assert!(
            LlamaCppEmbedder::new(EmbeddingProfile {
                endpoint: Some(valid.into()),
                ..base.clone()
            })
            .is_ok(),
            "rejected {valid}"
        );
    }
    for hostile in [
        "https://127.0.0.1:8082",
        "http://127.0.0.1",
        "http://127.0.0.1:0",
        "http://127.0.0.1:99999",
        "http://user@127.0.0.1:8082",
        "http://127.0.0.1:9@evil.example",
        "http://localhost.evil:8082",
        "http://127.0.0.2:8082",
        "http://127.1:8082",
        "http://2130706433:8082",
        "http://LOCALHOST:8082",
        "http://localhost:08082",
        "http://localhost:8082/",
        "http://localhost:8082/v1",
        "http://localhost:8082?query=1",
        "http://localhost:8082#fragment",
    ] {
        assert!(
            LlamaCppEmbedder::new(EmbeddingProfile {
                endpoint: Some(hostile.into()),
                ..base.clone()
            })
            .is_err(),
            "accepted {hostile}"
        );
    }
}

#[test]
fn status_has_no_text_and_context_uses_only_personal_schema() {
    let mut store = PersonalStore::open_in_memory().unwrap();
    store.capture(&capture("secret phrase", "1")).unwrap();
    let status = serde_json::to_string(
        &store
            .embedding_status(std::path::Path::new("/tmp/personal.db"))
            .unwrap(),
    )
    .unwrap();
    assert!(!status.contains("secret phrase"));
    assert!(status.contains("personal.db"));
    let context = store.local_context("secret", 10).unwrap();
    assert_eq!(context["records"][0]["text"], "secret phrase");
    let evidence_tables:i64=store.conn.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('memories','observations','artifacts')",[],|row|row.get(0)).unwrap();
    assert_eq!(evidence_tables, 0);
}
