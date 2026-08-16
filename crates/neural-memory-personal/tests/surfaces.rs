use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

use ed25519_dalek::SigningKey;
use neural_memory_personal::{
    identity, sign_payload, PersonalStore, SyncChange, SyncCursor, SyncPayloadV1, SyncRecord,
    SyncSighting, IDENTITY_DOMAIN,
};
use serde_json::{json, Value};
use tempfile::tempdir;

fn sync_command(directory: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_neural-memory-personal-sync"));
    command
        .env("NEURAL_MEMORY_PERSONAL_DB", directory.join("personal.db"))
        .env("NEURAL_MEMORY_PERSONAL_KEY", directory.join("sync.key"))
        .env("NEURAL_MEMORY_MAC_PUBLIC_KEY", directory.join("mac.pub"))
        .env("NEURAL_MEMORY_PERSONAL_DEVICE", "gpd");
    command
}

#[test]
fn sync_cli_creates_a_private_key_and_emits_json_only() {
    let directory = tempdir().unwrap();
    let output = sync_command(directory.path())
        .arg("public-key")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["algorithm"], "Ed25519");
    assert_eq!(response["signerKeyID"].as_str().unwrap().len(), 64);
    let mode = fs::metadata(directory.path().join("sync.key"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);

    let rejected = sync_command(directory.path())
        .args(["export", "--after", "1:0", "extra"])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    assert!(!rejected.stderr.is_empty());
}

#[test]
fn key_rotation_requires_confirmation_and_is_not_in_remote_grammar() {
    let directory = tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let first: Value = serde_json::from_slice(
        &sync_command(directory.path())
            .arg("public-key")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let rejected = sync_command(directory.path())
        .args(["rotate-key", "--confirm", "wrong"])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    let unchanged: Value = serde_json::from_slice(
        &sync_command(directory.path())
            .arg("public-key")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(first["signerKeyID"], unchanged["signerKeyID"]);
    let rotated = sync_command(directory.path())
        .args(["rotate-key", "--confirm", "ROTATE-SIGNING-KEY"])
        .output()
        .unwrap();
    assert!(rotated.status.success());
    let rotated: Value = serde_json::from_slice(&rotated.stdout).unwrap();
    assert_eq!(rotated["oldSignerKeyID"], first["signerKeyID"]);
    assert_ne!(rotated["newSignerKeyID"], first["signerKeyID"]);
}

#[test]
fn local_recovery_commands_require_existing_key_and_confirmation() {
    let directory = tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let missing = sync_command(directory.path())
        .args(["reset-replica", "--expected-predecessor-epoch", "0"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(!directory.path().join("personal.db").exists());

    assert!(sync_command(directory.path())
        .arg("public-key")
        .output()
        .unwrap()
        .status
        .success());
    let reset = sync_command(directory.path())
        .args(["reset-replica", "--expected-predecessor-epoch", "0"])
        .output()
        .unwrap();
    assert!(reset.status.success());
    let before: (i64, i64) = PersonalStore::open(&directory.path().join("personal.db"))
        .unwrap()
        .conn
        .query_row("SELECT epoch,sequence FROM promotion_state", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    let rejected = sync_command(directory.path())
        .args([
            "bump-epoch-and-snapshot",
            "--at",
            "2026-08-06T12:00:00.000Z",
            "--confirm",
            "wrong",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    let after: (i64, i64) = PersonalStore::open(&directory.path().join("personal.db"))
        .unwrap()
        .conn
        .query_row("SELECT epoch,sequence FROM promotion_state", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(before, after);

    let peer = SigningKey::from_bytes(&[12; 32]);
    let encoded = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        peer.verifying_key().as_bytes(),
    );
    let wrong = sync_command(directory.path())
        .args([
            "enroll-peer",
            "--public-key-base64",
            &encoded,
            "--confirm",
            "wrong",
        ])
        .output()
        .unwrap();
    assert!(!wrong.status.success());
    assert!(!directory.path().join("mac.pub").exists());
    let enrolled = sync_command(directory.path())
        .args([
            "enroll-peer",
            "--public-key-base64",
            &encoded,
            "--confirm",
            "ENROLL-MAC-PEER",
        ])
        .output()
        .unwrap();
    assert!(enrolled.status.success());
    assert_eq!(
        fs::metadata(directory.path().join("mac.pub"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn sync_import_prints_ack_after_the_bundle_commits() {
    let directory = tempdir().unwrap();
    let signing = SigningKey::from_bytes(&[9; 32]);
    let digest = identity("canonical memory", None, "{}").unwrap().0;
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
        generated_at: "2026-08-06T12:00:01.000Z".into(),
        changes: vec![SyncChange {
            cursor,
            operation: "upsert".into(),
            record: SyncRecord {
                content_domain: IDENTITY_DOMAIN.into(),
                content_digest: digest.clone(),
                text: "canonical memory".into(),
                occurred_at: None,
                metadata: json!({}),
                created_at: "2026-08-06T12:00:00.000Z".into(),
                tombstoned: false,
                tags: vec!["canonical".into()],
                sightings: vec![SyncSighting {
                    origin_device: "mac".into(),
                    origin_record_id: "mac-1".into(),
                    captured_at: "2026-08-06T12:00:00.000Z".into(),
                    source: None,
                    conversation_id: None,
                }],
            },
        }],
        divergences: vec![],
    };
    let envelope = sign_payload(&payload, &signing).unwrap();
    fs::write(
        directory.path().join("mac.pub"),
        signing.verifying_key().as_bytes(),
    )
    .unwrap();
    fs::set_permissions(
        directory.path().join("mac.pub"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let mut child = sync_command(directory.path())
        .arg("import")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    serde_json::to_writer(child.stdin.as_mut().unwrap(), &envelope).unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"ack":{"through":{"epoch":1,"sequence":1},"committed":true}})
    );
    let store = PersonalStore::open(&directory.path().join("personal.db")).unwrap();
    let stored_cursor = store.cursor().unwrap();
    assert_eq!((stored_cursor.epoch, stored_cursor.sequence), (1, 1));
    assert!(store.get(&digest).unwrap().is_some());
}

#[test]
fn mcp_stdio_lists_only_personal_tools_and_keeps_stdout_json() {
    let directory = tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_neural-memory-personal-mcp"))
        .args([
            "--db",
            directory.path().join("personal.db").to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
        .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    let names: Vec<_> = response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["remember", "recall", "list_recent", "forget"]);
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("personal.db"));
}

#[test]
fn forced_command_accepts_only_versioned_sync_grammar() {
    let directory = tempdir().unwrap();
    let mock = directory.path().join("mock-sync");
    fs::write(&mock, "#!/bin/sh\nprintf '{\"args\":\"%s\"}\\n' \"$*\"\n").unwrap();
    fs::set_permissions(&mock, fs::Permissions::from_mode(0o700)).unwrap();
    let wrapper_source = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/personal-sync-forced-command-v1.sh"
    );
    let mountpoint = directory.path().join("mountpoint");
    fs::write(&mountpoint, "#!/bin/sh\n[ \"${TEST_MOUNT_READY-}\" = 1 ]\n").unwrap();
    fs::set_permissions(&mountpoint, fs::Permissions::from_mode(0o700)).unwrap();
    let wrapper = directory.path().join("personal-sync-wrapper");
    fs::write(
        &wrapper,
        fs::read_to_string(wrapper_source)
            .unwrap()
            .replace("/usr/bin/mountpoint", mountpoint.to_str().unwrap()),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let run = |original: &str, mounted: bool| {
        Command::new(&wrapper)
            .env("NEURAL_MEMORY_PERSONAL_SYNC_BIN", &mock)
            .env(
                "NEURAL_MEMORY_PERSONAL_DB",
                directory.path().join("personal.db"),
            )
            .env(
                "NEURAL_MEMORY_PERSONAL_KEY",
                directory.path().join("sync.key"),
            )
            .env(
                "NEURAL_MEMORY_MAC_PUBLIC_KEY",
                directory.path().join("mac.pub"),
            )
            .env("NEURAL_MEMORY_PERSONAL_DEVICE", "gpd")
            .env("SSH_ORIGINAL_COMMAND", original)
            .env("TEST_MOUNT_READY", if mounted { "1" } else { "0" })
            .output()
            .unwrap()
    };
    let status = run("status", true);
    assert!(status.status.success());
    assert_eq!(status.stdout, b"{\"health\":\"ready\"}\n");
    let blocked_status = run("status", false);
    assert!(blocked_status.status.success());
    assert_eq!(
        blocked_status.stdout,
        b"{\"health\":\"blocked-on-mount\"}\n"
    );
    let allowed = run("export --after 1:9", true);
    assert!(allowed.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&allowed.stdout).unwrap()["args"],
        "export --after 1:9"
    );
    for blocked in ["export --after 1:9", "import"] {
        let output = run(blocked, false);
        assert_eq!(output.status.code(), Some(75));
        assert!(output.stdout.is_empty(), "child ran for {blocked}");
        assert!(String::from_utf8(output.stderr)
            .unwrap()
            .contains("personal data mount unavailable"));
    }
    for rejected in [
        "recall",
        "import --trusted-key-base64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "reset-replica --expected-predecessor-epoch 1",
        "rotate-key --confirm ROTATE-SIGNING-KEY",
        "enroll-peer --public-key-base64 AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA= --confirm ENROLL-MAC-PEER",
        "bump-epoch-and-snapshot --at 2026-08-06T00:00:00.000Z --confirm BUMP-EPOCH-AND-SNAPSHOT",
        "export --after 1:9; id",
        "export --after 1:9 extra",
    ] {
        let output = run(rejected, true);
        assert!(!output.status.success(), "accepted {rejected}");
        assert!(output.stdout.is_empty());
    }
}
