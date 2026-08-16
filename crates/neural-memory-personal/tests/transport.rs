use std::fs;
use std::os::unix::fs::PermissionsExt;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::SigningKey;
use neural_memory_personal::runtime::load_bearer_token;
use neural_memory_personal::transport::{
    handle, require_loopback, TransportConfig, MAX_BODY_BYTES,
};
use neural_memory_personal::{
    identity, sign_payload, PersonalStore, SyncChange, SyncCursor, SyncPayloadV1, SyncRecord,
    SyncSighting, IDENTITY_DOMAIN,
};
use serde_json::{json, Value};
use tempfile::tempdir;

const TOKEN: &[u8] = b"KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio=";
const AUTHORIZATION: &str = "Bearer KioqKioqKioqKioqKioqKioqKioqKioqKioqKioqKio=";
const T0: &str = "2026-08-06T12:00:00.000Z";
const T1: &str = "2026-08-06T12:00:01.000Z";

fn config(directory: &std::path::Path) -> TransportConfig {
    TransportConfig {
        database: directory.join("personal.db"),
        signing_key: directory.join("signing.key"),
        peer_key: directory.join("mac.pub"),
        source_device: "gpd".into(),
    }
}

fn request(config: &TransportConfig, body: Value) -> neural_memory_personal::transport::Response {
    handle(
        config,
        "POST",
        "/v1/personal-sync",
        Some(AUTHORIZATION),
        TOKEN,
        &serde_json::to_vec(&body).unwrap(),
    )
}

#[test]
fn method_path_token_and_action_grammar_are_closed() {
    let directory = tempdir().unwrap();
    let config = config(directory.path());
    let valid = serde_json::to_vec(&json!({"action":"publicKey"})).unwrap();
    assert_eq!(
        handle(
            &config,
            "GET",
            "/v1/personal-sync",
            Some(AUTHORIZATION),
            TOKEN,
            &valid
        )
        .status,
        405
    );
    assert_eq!(
        handle(
            &config,
            "POST",
            "/v1/personal-sync/",
            Some(AUTHORIZATION),
            TOKEN,
            &valid
        )
        .status,
        404
    );
    for credential in [None, Some("Bearer wrong"), Some("Basic anything")] {
        assert_eq!(
            handle(
                &config,
                "POST",
                "/v1/personal-sync",
                credential,
                TOKEN,
                &valid
            )
            .status,
            401
        );
    }
    for body in [
        json!({"action":"recall"}),
        json!({"action":"mcp"}),
        json!({"action":"resetReplica"}),
        json!({"action":"rotateKey"}),
        json!({"action":"sql","query":"select 1"}),
        json!({"action":"publicKey","shell":"$(id)"}),
        json!({"action":"export","after":"1:0;id"}),
        json!({"action":"export","after":"1:0","path":"/etc/passwd"}),
        json!({"action":"import","trustedKeyBase64":"x","envelope":{"version":"SyncBundleV1","algorithm":"Ed25519","signerKeyID":"x","payloadBase64":"x","signatureBase64":"x"}}),
    ] {
        assert_eq!(request(&config, body).status, 400);
    }
    assert_eq!(request(&config, json!({"action":"publicKey"})).status, 200);
}

#[test]
fn authenticated_status_is_exact_and_does_not_open_data_files() {
    let directory = tempdir().unwrap();
    let unavailable = directory.path().join("unavailable");
    let config = TransportConfig {
        database: unavailable.join("personal.db"),
        signing_key: unavailable.join("signing.key"),
        peer_key: unavailable.join("mac.pub"),
        source_device: "gpd".into(),
    };
    let response = request(&config, json!({"action":"status"}));
    assert_eq!(response.status, 200);
    assert_eq!(response.body, json!({"health":"ready"}));
    assert!(!unavailable.exists());

    assert_eq!(
        request(&config, json!({"action":"status","path":"personal.db"})).status,
        400
    );
    assert_eq!(
        handle(
            &config,
            "POST",
            "/v1/personal-sync",
            None,
            TOKEN,
            br#"{"action":"status"}"#,
        )
        .status,
        401
    );
}

#[test]
fn only_loopback_listeners_are_accepted() {
    assert!(require_loopback("127.0.0.1:9443".parse().unwrap()).is_ok());
    assert!(require_loopback("[::1]:9443".parse().unwrap()).is_ok());
    assert!(require_loopback("0.0.0.0:9443".parse().unwrap()).is_err());
    assert!(require_loopback("192.0.2.1:9443".parse().unwrap()).is_err());
}

#[test]
fn body_limit_is_enforced_before_json_parsing() {
    let directory = tempdir().unwrap();
    let oversized = vec![b' '; MAX_BODY_BYTES + 1];
    let response = handle(
        &config(directory.path()),
        "POST",
        "/v1/personal-sync",
        Some(AUTHORIZATION),
        TOKEN,
        &oversized,
    );
    assert_eq!(response.status, 413);
    assert_eq!(response.body["error"]["code"], "bodyTooLarge");
}

#[test]
fn bearer_token_file_requires_mode_0600_and_high_entropy_length() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("transport.token");
    fs::write(&path, TOKEN).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(load_bearer_token(&path).unwrap(), TOKEN);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    assert!(load_bearer_token(&path).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, b"too-short").unwrap();
    assert!(load_bearer_token(&path).is_err());
}

fn import_body(signing: &SigningKey) -> (Value, String) {
    let digest = identity("from Mac", None, "{}").unwrap().0;
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
                content_digest: digest.clone(),
                text: "from Mac".into(),
                occurred_at: None,
                metadata: json!({}),
                created_at: T0.into(),
                tombstoned: false,
                tags: vec!["canonical".into()],
                sightings: vec![SyncSighting {
                    origin_device: "mac".into(),
                    origin_record_id: "mac-1".into(),
                    captured_at: T0.into(),
                    source: None,
                    conversation_id: None,
                }],
            },
        }],
        divergences: vec![],
    };
    let envelope = sign_payload(&payload, signing).unwrap();
    (
        json!({
            "action":"import",
            "envelope":envelope
        }),
        digest,
    )
}

#[test]
fn import_rejects_tampering_and_acknowledges_only_committed_state() {
    let directory = tempdir().unwrap();
    let config = config(directory.path());
    let signing = SigningKey::from_bytes(&[11; 32]);
    fs::write(&config.peer_key, signing.verifying_key().as_bytes()).unwrap();
    fs::set_permissions(&config.peer_key, fs::Permissions::from_mode(0o600)).unwrap();
    let (body, digest) = import_body(&signing);
    let mut tampered = body.clone();
    tampered["envelope"]["payloadBase64"] = json!(BASE64.encode(b"{}"));
    assert_eq!(request(&config, tampered).status, 400);
    let untouched = PersonalStore::open(&config.database).unwrap();
    assert_eq!(untouched.cursor().unwrap().sequence, 0);
    assert!(untouched.get(&digest).unwrap().is_none());
    drop(untouched);

    let response = request(&config, body);
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body,
        json!({"ack":{"through":{"epoch":1,"sequence":1},"committed":true}})
    );
    let committed = PersonalStore::open(&config.database).unwrap();
    assert_eq!(committed.cursor().unwrap().sequence, 1);
    assert!(committed.get(&digest).unwrap().is_some());
    let other = SigningKey::from_bytes(&[12; 32]);
    let (request_selected, _) = import_body(&other);
    assert_eq!(request(&config, request_selected).status, 400);
}
