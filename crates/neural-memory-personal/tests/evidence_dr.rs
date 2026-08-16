use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::process::Command;

use ed25519_dalek::SigningKey;
use neural_memory_personal::evidence_dr::{
    accept, read_current, stage, verify_directory, StageConfig, ARTIFACT_NAME, MANIFEST_NAME,
    SIGNATURE_NAME,
};
use tempfile::tempdir;

const CREATED_AT: &str = "2026-08-06T12:34:56.789Z";

fn mock_admin(directory: &std::path::Path, verify_success: bool) -> std::path::PathBuf {
    let path = directory.join("mock-admin");
    let verify = if verify_success { "exit 0" } else { "exit 1" };
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nset -eu\nif [ \"$1\" = backup ]; then\n  printf 'verified evidence' >\"$5\"\n  echo \"backed up to $5 (17 bytes, 2 records, 3 observations, 4 edges, schema 1) -- verified: every record digest, observation identity and provenance edge matches\"\n  exit 0\nfi\nif [ \"$1\" = verify-backup ]; then\n  echo \"$5 matches $3 exactly\"\n  {verify}\nfi\nexit 2\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn staged(verify_success: bool) -> (tempfile::TempDir, SigningKey) {
    let directory = tempdir().unwrap();
    let root = directory.path().join("dr");
    fs::create_dir(&root).unwrap();
    let source = directory.path().join("store.db");
    fs::write(&source, b"source is retained").unwrap();
    let admin = mock_admin(directory.path(), verify_success);
    let key = SigningKey::from_bytes(&[23; 32]);
    let result = stage(&StageConfig {
        root: &root,
        source: &source,
        admin: &admin,
        created_at: CREATED_AT,
        signing_key: &key,
    });
    if verify_success {
        let manifest = result.unwrap();
        assert_eq!(manifest.source_counts, manifest.backup_counts);
        assert_eq!(manifest.source_counts.records, 2);
        assert!(source.exists(), "staging must retain the source");
    } else {
        assert!(result.is_err());
    }
    (directory, key)
}

#[test]
fn manifest_is_not_signed_before_both_admin_verifications_succeed() {
    let (directory, _) = staged(false);
    let staging = directory.path().join("dr/staging");
    assert!(staging.join(ARTIFACT_NAME).exists());
    assert!(!staging.join(MANIFEST_NAME).exists());
    assert!(!staging.join(SIGNATURE_NAME).exists());
}

#[test]
fn tampered_signature_and_hash_or_size_are_rejected() {
    let (directory, key) = staged(true);
    let staging = directory.path().join("dr/staging");
    verify_directory(&staging, &key.verifying_key()).unwrap();

    let signature_path = staging.join(SIGNATURE_NAME);
    let original_signature = fs::read(&signature_path).unwrap();
    let mut envelope: serde_json::Value = serde_json::from_slice(&original_signature).unwrap();
    envelope["signatureBase64"] = serde_json::json!(
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="
    );
    fs::write(&signature_path, serde_json::to_vec(&envelope).unwrap()).unwrap();
    assert!(verify_directory(&staging, &key.verifying_key()).is_err());
    fs::write(&signature_path, original_signature).unwrap();

    let artifact_path = staging.join(ARTIFACT_NAME);
    let original_artifact = fs::read(&artifact_path).unwrap();
    fs::write(&artifact_path, b"verified evidence plus").unwrap();
    assert!(verify_directory(&staging, &key.verifying_key()).is_err());
    let mut same_size = original_artifact.clone();
    same_size[0] ^= 1;
    fs::write(&artifact_path, same_size).unwrap();
    assert!(verify_directory(&staging, &key.verifying_key()).is_err());
}

#[test]
fn acceptance_is_signature_checked_atomic_and_keeps_staging() {
    let (directory, key) = staged(true);
    let root = directory.path().join("dr");
    accept(&root, &key.verifying_key()).unwrap();
    assert!(root.join("staging").is_dir());
    assert_eq!(
        read_current(&root, ARTIFACT_NAME).unwrap(),
        b"verified evidence"
    );
    fs::write(root.join("staging").join(ARTIFACT_NAME), b"changed staging").unwrap();
    assert_eq!(
        read_current(&root, ARTIFACT_NAME).unwrap(),
        b"verified evidence"
    );
    assert!(accept(&root, &key.verifying_key()).is_err());
}

#[test]
fn pull_rejects_paths_and_symlinks() {
    let (directory, key) = staged(true);
    let root = directory.path().join("dr");
    accept(&root, &key.verifying_key()).unwrap();
    assert!(read_current(&root, "../personal.db").is_err());
    let artifact = root.join("current").join(ARTIFACT_NAME);
    fs::remove_file(&artifact).unwrap();
    symlink("/etc/passwd", &artifact).unwrap();
    assert!(read_current(&root, ARTIFACT_NAME).is_err());
}

#[test]
fn deployment_binary_rejects_personal_sources_and_malformed_pull_commands() {
    let binary = env!("CARGO_BIN_EXE_neural-memory-evidence-dr");
    let base = || {
        let mut command = Command::new(binary);
        command
            .env(
                "NEURAL_MEMORY_EVIDENCE_DR_DIR",
                "/srv/neural-memory-data/backups/evidence-dr",
            )
            .env(
                "NEURAL_MEMORY_EVIDENCE_SOURCE",
                "/srv/neural-memory-data/personal/personal.db",
            )
            .env(
                "NEURAL_MEMORY_EVIDENCE_ADMIN",
                "/usr/local/bin/neural-memory-admin",
            )
            .env(
                "NEURAL_MEMORY_PERSONAL_KEY",
                "/srv/neural-memory-data/keys/gpd-ed25519.seed",
            )
            .env("NEURAL_MEMORY_EVIDENCE_WRITERS_STOPPED", "1");
        command
    };
    let rejected = base()
        .args(["stage", "--writers-stopped", "--created-at", CREATED_AT])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8(rejected.stderr)
        .unwrap()
        .contains("evidence source"));

    for args in [
        vec!["list", "extra"],
        vec!["stream", "../store.db"],
        vec!["recall"],
    ] {
        let output = base().args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
