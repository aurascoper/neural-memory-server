use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use ed25519_dalek::SigningKey;
use neural_memory_personal::interaction_logs_dr::{
    accept, read_current, stage, verify_directory, StageConfig, ARTIFACT_NAME, MANIFEST_NAME,
    SIGNATURE_NAME,
};
use tempfile::tempdir;

const CREATED_AT: &str = "2026-08-16T12:34:56.789Z";
const TAR: &str = "/usr/bin/tar";

fn fixture_source(directory: &Path) -> std::path::PathBuf {
    let source = directory.join("interaction-logs");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("s1.eeg.jsonl"), b"{\"sequence\":1}\n").unwrap();
    fs::write(source.join("s1.turns.jsonl"), b"{\"index\":0}\n").unwrap();
    let workspace = source.join("s1.workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("manifest.json"), b"{}\n").unwrap();
    source
}

fn staged() -> (tempfile::TempDir, SigningKey) {
    let directory = tempdir().unwrap();
    let root = directory.path().join("dr");
    fs::create_dir(&root).unwrap();
    let source = fixture_source(directory.path());
    let key = SigningKey::from_bytes(&[42; 32]);
    let manifest = stage(&StageConfig {
        root: &root,
        source: &source,
        tar: Path::new(TAR),
        created_at: CREATED_AT,
        signing_key: &key,
    })
    .unwrap();
    assert_eq!(manifest.file_count, 3);
    assert_eq!(
        manifest.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        ["s1.eeg.jsonl", "s1.turns.jsonl", "s1.workspace/manifest.json"]
    );
    assert_eq!(
        manifest.content_bytes,
        manifest.files.iter().map(|f| f.byte_length).sum::<u64>()
    );
    assert!(source.exists(), "staging must retain the source");
    (directory, key)
}

#[test]
fn stage_verify_accept_round_trip() {
    let (directory, key) = staged();
    let root = directory.path().join("dr");
    let manifest = verify_directory(&root.join("staging"), &key.verifying_key()).unwrap();
    let accepted = accept(&root, &key.verifying_key()).unwrap();
    assert_eq!(manifest, accepted);
    for name in [ARTIFACT_NAME, MANIFEST_NAME, SIGNATURE_NAME] {
        assert!(!read_current(&root, name).unwrap().is_empty());
    }
    // second acceptance must refuse to replace current
    assert!(accept(&root, &key.verifying_key())
        .unwrap_err()
        .contains("current directory already exists"));
}

#[test]
fn deterministic_archive_for_identical_content() {
    let (directory_a, _) = staged();
    let (directory_b, _) = staged();
    let read = |directory: &tempfile::TempDir| {
        fs::read(directory.path().join("dr/staging").join(ARTIFACT_NAME)).unwrap()
    };
    assert_eq!(read(&directory_a), read(&directory_b));
}

#[test]
fn tampered_archive_is_rejected() {
    let (directory, key) = staged();
    let artifact = directory.path().join("dr/staging").join(ARTIFACT_NAME);
    let mut bytes = fs::read(&artifact).unwrap();
    let index = bytes.len() / 2;
    bytes[index] ^= 0xff;
    fs::write(&artifact, bytes).unwrap();
    let error =
        verify_directory(&directory.path().join("dr/staging"), &key.verifying_key()).unwrap_err();
    assert!(error.contains("SHA-256 mismatch"), "{error}");
}

#[test]
fn tampered_manifest_fails_signature() {
    let (directory, key) = staged();
    let manifest_path = directory.path().join("dr/staging").join(MANIFEST_NAME);
    let text = String::from_utf8(fs::read(&manifest_path).unwrap()).unwrap();
    fs::write(&manifest_path, text.replace(CREATED_AT, "2026-08-16T12:34:56.790Z")).unwrap();
    let error =
        verify_directory(&directory.path().join("dr/staging"), &key.verifying_key()).unwrap_err();
    assert!(error.contains("identity mismatch"), "{error}");
}

#[test]
fn wrong_key_is_rejected() {
    let (directory, _key) = staged();
    let other = SigningKey::from_bytes(&[7; 32]);
    let error =
        verify_directory(&directory.path().join("dr/staging"), &other.verifying_key()).unwrap_err();
    assert!(error.contains("identity mismatch"), "{error}");
}

#[test]
fn symlink_in_source_refuses_to_stage() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("dr");
    fs::create_dir(&root).unwrap();
    let source = fixture_source(directory.path());
    symlink("s1.eeg.jsonl", source.join("alias.jsonl")).unwrap();
    let key = SigningKey::from_bytes(&[42; 32]);
    let error = stage(&StageConfig {
        root: &root,
        source: &source,
        tar: Path::new(TAR),
        created_at: CREATED_AT,
        signing_key: &key,
    })
    .unwrap_err();
    assert!(error.contains("symlink in source tree"), "{error}");
}

#[test]
fn empty_source_refuses_to_stage() {
    let directory = tempdir().unwrap();
    let root = directory.path().join("dr");
    fs::create_dir(&root).unwrap();
    let source = directory.path().join("interaction-logs");
    fs::create_dir(&source).unwrap();
    let key = SigningKey::from_bytes(&[42; 32]);
    let error = stage(&StageConfig {
        root: &root,
        source: &source,
        tar: Path::new(TAR),
        created_at: CREATED_AT,
        signing_key: &key,
    })
    .unwrap_err();
    assert!(error.contains("refusing to stage nothing"), "{error}");
}

#[test]
fn existing_staging_refuses() {
    let (directory, key) = staged();
    let root = directory.path().join("dr");
    let source = directory.path().join("interaction-logs");
    let error = stage(&StageConfig {
        root: &root,
        source: &source,
        tar: Path::new(TAR),
        created_at: CREATED_AT,
        signing_key: &key,
    })
    .unwrap_err();
    assert!(error.contains("staging directory already exists"), "{error}");
}
