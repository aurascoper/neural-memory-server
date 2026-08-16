//! Signed disaster-recovery staging for the interaction logs.
//!
//! Sibling of [`crate::evidence_dr`], same contract shape, different artifact:
//! a deterministic GNU tar of `/srv/neural-memory-data/interaction-logs` (raw
//! EEG captures, turn logs, session records, and the derived `.workspace/`
//! artifacts). The manifest carries a per-file digest listing so the Mac can
//! verify individual members after extraction, not just the archive.
//!
//! Staging verifies by EXTRACTION: the archive is unpacked into a scratch
//! directory and every file's SHA-256 is compared against the pre-tar walk.
//! That double hash is also the writers check in practice — a capture growing
//! between the walk and the tar shows up as a digest mismatch and fails the
//! stage, leaving the staging directory for inspection.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::evidence_dr::{
    copy_private, create_private, ensure_real_directory, read_fixed, safe_metadata, sha256_file,
};
use crate::{canonical_timestamp, signer_key_id};

pub const ARTIFACT_NAME: &str = "interaction-logs-current.tar";
pub const MANIFEST_NAME: &str = "interaction-logs-current.manifest.json";
pub const SIGNATURE_NAME: &str = "interaction-logs-current.manifest.sig.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileEntry {
    pub path: String,
    pub byte_length: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractionLogsManifestV1 {
    pub version: String,
    pub artifact_filename: String,
    pub byte_length: u64,
    pub sha256: String,
    pub created_at: String,
    pub file_count: u64,
    pub content_bytes: u64,
    pub files: Vec<FileEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedInteractionLogsManifestV1 {
    pub version: String,
    pub algorithm: String,
    pub signer_key_id: String,
    pub payload_base64: String,
    pub signature_base64: String,
}

pub struct StageConfig<'a> {
    pub root: &'a Path,
    pub source: &'a Path,
    pub tar: &'a Path,
    pub created_at: &'a str,
    pub signing_key: &'a SigningKey,
}

/// Walk `source` recursively: sorted relative paths -> (byte length, sha256).
/// Symlinks anywhere in the tree are an error, never silently followed or
/// skipped — a symlink in a capture directory is a surprise worth failing on.
fn walk(source: &Path) -> Result<BTreeMap<String, (u64, String)>, String> {
    ensure_real_directory(source)?;
    let mut out = BTreeMap::new();
    let mut stack = vec![source.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|error| error.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if metadata.file_type().is_symlink() {
                return Err(format!("symlink in source tree: {}", path.display()));
            }
            if metadata.file_type().is_dir() {
                stack.push(path);
                continue;
            }
            if !metadata.file_type().is_file() {
                return Err(format!("non-regular file in source tree: {}", path.display()));
            }
            let rel = path
                .strip_prefix(source)
                .map_err(|error| error.to_string())?
                .to_str()
                .ok_or_else(|| format!("non-UTF-8 path: {}", path.display()))?
                .to_string();
            out.insert(rel, (metadata.len(), sha256_file(&path)?));
        }
    }
    Ok(out)
}

pub fn stage(config: &StageConfig<'_>) -> Result<InteractionLogsManifestV1, String> {
    if !canonical_timestamp(config.created_at) {
        return Err("createdAt must be canonical UTC with exactly milliseconds".into());
    }
    let staging = config.root.join("staging");
    ensure_real_directory(config.root)?;
    if staging.exists() {
        return Err("fixed staging directory already exists".into());
    }
    fs::create_dir(&staging).map_err(|error| error.to_string())?;
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let artifact = staging.join(ARTIFACT_NAME);

    let walked = walk(config.source)?;
    if walked.is_empty() {
        return Err("interaction-logs source is empty; refusing to stage nothing".into());
    }

    // Deterministic GNU tar: identical content produces an identical archive.
    let tar = Command::new(config.tar)
        .args([
            "--format=gnu",
            "--sort=name",
            "--mtime=@0",
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "-cf",
        ])
        .arg(&artifact)
        .arg("-C")
        .arg(config.source)
        .arg(".")
        .output()
        .map_err(|error| format!("run tar: {error}"))?;
    if !tar.status.success() {
        return Err("tar archive creation failed".into());
    }

    // Verify by extraction: unpack and compare every digest against the walk.
    // A file that changed between walk and tar fails here.
    let scratch = staging.join(".verify");
    fs::create_dir(&scratch).map_err(|error| error.to_string())?;
    fs::set_permissions(&scratch, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let untar = Command::new(config.tar)
        .arg("-xf")
        .arg(&artifact)
        .arg("-C")
        .arg(&scratch)
        .output()
        .map_err(|error| format!("run tar extraction: {error}"))?;
    if !untar.status.success() {
        return Err("tar extraction for verification failed".into());
    }
    let extracted = walk(&scratch)?;
    if extracted != walked {
        return Err(
            "extracted archive does not match the source walk (a writer may be live)".into(),
        );
    }
    fs::remove_dir_all(&scratch).map_err(|error| error.to_string())?;

    let metadata = safe_metadata(&artifact)?;
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    let files: Vec<FileEntry> = walked
        .iter()
        .map(|(path, (byte_length, sha256))| FileEntry {
            path: path.clone(),
            byte_length: *byte_length,
            sha256: sha256.clone(),
        })
        .collect();
    let manifest = InteractionLogsManifestV1 {
        version: "InteractionLogsBackupManifestV1".into(),
        artifact_filename: ARTIFACT_NAME.into(),
        byte_length: metadata.len(),
        sha256: sha256_file(&artifact)?,
        created_at: config.created_at.into(),
        file_count: files.len() as u64,
        content_bytes: files.iter().map(|file| file.byte_length).sum(),
        files,
    };
    let bytes = serde_json::to_vec(&manifest).map_err(|error| error.to_string())?;
    let envelope = sign_manifest(&bytes, config.signing_key);
    create_private(&staging.join(MANIFEST_NAME), &bytes)?;
    create_private(
        &staging.join(SIGNATURE_NAME),
        &serde_json::to_vec(&envelope).map_err(|error| error.to_string())?,
    )?;
    Ok(manifest)
}

pub fn accept(
    root: &Path,
    enrolled_key: &VerifyingKey,
) -> Result<InteractionLogsManifestV1, String> {
    let staging = root.join("staging");
    let current = root.join("current");
    ensure_real_directory(root)?;
    ensure_real_directory(&staging)?;
    if current.exists() {
        return Err("fixed current directory already exists; retain it before replacement".into());
    }
    let manifest = verify_directory(&staging, enrolled_key)?;
    let next = root.join("current.next");
    if next.exists() {
        return Err("fixed current.next directory already exists".into());
    }
    fs::create_dir(&next).map_err(|error| error.to_string())?;
    fs::set_permissions(&next, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    for name in [ARTIFACT_NAME, MANIFEST_NAME, SIGNATURE_NAME] {
        copy_private(&staging.join(name), &next.join(name))?;
    }
    verify_directory(&next, enrolled_key)?;
    fs::rename(&next, &current).map_err(|error| error.to_string())?;
    Ok(manifest)
}

pub fn verify_directory(
    directory: &Path,
    enrolled_key: &VerifyingKey,
) -> Result<InteractionLogsManifestV1, String> {
    ensure_real_directory(directory)?;
    let manifest_bytes = read_fixed(directory, MANIFEST_NAME)?;
    let envelope_bytes = read_fixed(directory, SIGNATURE_NAME)?;
    let envelope: SignedInteractionLogsManifestV1 =
        serde_json::from_slice(&envelope_bytes).map_err(|error| error.to_string())?;
    verify_manifest_signature(&manifest_bytes, &envelope, enrolled_key)?;
    let manifest: InteractionLogsManifestV1 =
        serde_json::from_slice(&manifest_bytes).map_err(|error| error.to_string())?;
    validate_manifest(&manifest)?;
    let artifact = directory.join(ARTIFACT_NAME);
    let metadata = safe_metadata(&artifact)?;
    if metadata.len() != manifest.byte_length {
        return Err("archive byte length mismatch".into());
    }
    if sha256_file(&artifact)? != manifest.sha256 {
        return Err("archive SHA-256 mismatch".into());
    }
    Ok(manifest)
}

pub fn read_current(root: &Path, name: &str) -> Result<Vec<u8>, String> {
    if !matches!(name, ARTIFACT_NAME | MANIFEST_NAME | SIGNATURE_NAME) {
        return Err("unknown fixed artifact".into());
    }
    ensure_real_directory(root)?;
    let current = root.join("current");
    ensure_real_directory(&current)?;
    read_fixed(&current, name)
}

fn sign_manifest(bytes: &[u8], key: &SigningKey) -> SignedInteractionLogsManifestV1 {
    SignedInteractionLogsManifestV1 {
        version: "SignedInteractionLogsBackupManifestV1".into(),
        algorithm: "Ed25519".into(),
        signer_key_id: signer_key_id(&key.verifying_key()),
        payload_base64: BASE64.encode(bytes),
        signature_base64: BASE64.encode(key.sign(bytes).to_bytes()),
    }
}

fn verify_manifest_signature(
    literal_manifest: &[u8],
    envelope: &SignedInteractionLogsManifestV1,
    key: &VerifyingKey,
) -> Result<(), String> {
    if envelope.version != "SignedInteractionLogsBackupManifestV1"
        || envelope.algorithm != "Ed25519"
        || envelope.signer_key_id != signer_key_id(key)
        || BASE64
            .decode(&envelope.payload_base64)
            .map_err(|_| "invalid payload base64")?
            != literal_manifest
    {
        return Err("signed manifest identity mismatch".into());
    }
    let signature = BASE64
        .decode(&envelope.signature_base64)
        .map_err(|_| "invalid signature base64")?;
    let signature = Signature::from_slice(&signature).map_err(|_| "invalid signature")?;
    key.verify(literal_manifest, &signature)
        .map_err(|_| "invalid manifest signature".into())
}

fn lowercase_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_manifest(manifest: &InteractionLogsManifestV1) -> Result<(), String> {
    if manifest.version != "InteractionLogsBackupManifestV1"
        || manifest.artifact_filename != ARTIFACT_NAME
        || !canonical_timestamp(&manifest.created_at)
        || !lowercase_hex_sha256(&manifest.sha256)
        || manifest.file_count == 0
        || manifest.file_count != manifest.files.len() as u64
        || manifest.content_bytes
            != manifest
                .files
                .iter()
                .map(|file| file.byte_length)
                .sum::<u64>()
    {
        return Err("invalid interaction-logs manifest".into());
    }
    for file in &manifest.files {
        let path: &Path = Path::new(&file.path);
        if path.is_absolute()
            || file.path.split('/').any(|part| part == ".." || part.is_empty())
            || !lowercase_hex_sha256(&file.sha256)
        {
            return Err("invalid interaction-logs manifest file entry".into());
        }
    }
    Ok(())
}
