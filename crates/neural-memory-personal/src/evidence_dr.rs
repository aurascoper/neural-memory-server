use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{canonical_timestamp, signer_key_id};

pub const ARTIFACT_NAME: &str = "evidence-current.db";
pub const MANIFEST_NAME: &str = "evidence-current.manifest.json";
pub const SIGNATURE_NAME: &str = "evidence-current.manifest.sig.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceCounts {
    pub records: u64,
    pub observations: u64,
    pub edges: u64,
    pub schema_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceManifestV1 {
    pub version: String,
    pub artifact_filename: String,
    pub byte_length: u64,
    pub sha256: String,
    pub created_at: String,
    pub evidence_verifier_success: bool,
    pub source_counts: EvidenceCounts,
    pub backup_counts: EvidenceCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedEvidenceManifestV1 {
    pub version: String,
    pub algorithm: String,
    #[serde(rename = "signerKeyID")]
    pub signer_key_id: String,
    pub payload_base64: String,
    pub signature_base64: String,
}

pub struct StageConfig<'a> {
    pub root: &'a Path,
    pub source: &'a Path,
    pub admin: &'a Path,
    pub created_at: &'a str,
    pub signing_key: &'a SigningKey,
}

pub fn stage(config: &StageConfig<'_>) -> Result<EvidenceManifestV1, String> {
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

    let backup = Command::new(config.admin)
        .args(["backup", "--db"])
        .arg(config.source)
        .arg("--to")
        .arg(&artifact)
        .output()
        .map_err(|error| format!("run evidence backup: {error}"))?;
    if !backup.status.success() {
        return Err("evidence backup or built-in verification failed".into());
    }
    let report = String::from_utf8(backup.stdout).map_err(|_| "admin output is not UTF-8")?;
    if !report.contains("-- verified:") {
        return Err("admin backup did not report verifier success".into());
    }
    let source_counts = parse_counts(&report)?;

    let verify = Command::new(config.admin)
        .args(["verify-backup", "--db"])
        .arg(config.source)
        .arg("--of")
        .arg(&artifact)
        .output()
        .map_err(|error| format!("run evidence verifier: {error}"))?;
    if !verify.status.success() {
        return Err("explicit evidence backup verification failed".into());
    }
    let verified = String::from_utf8(verify.stdout).map_err(|_| "verifier output is not UTF-8")?;
    if !verified.contains(" matches ") || !verified.contains(" exactly") {
        return Err("evidence verifier did not report an exact match".into());
    }

    let metadata = fs::metadata(&artifact).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("backup artifact is not a regular file".into());
    }
    fs::set_permissions(&artifact, fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    let manifest = EvidenceManifestV1 {
        version: "EvidenceBackupManifestV1".into(),
        artifact_filename: ARTIFACT_NAME.into(),
        byte_length: metadata.len(),
        sha256: sha256_file(&artifact)?,
        created_at: config.created_at.into(),
        evidence_verifier_success: true,
        source_counts: source_counts.clone(),
        backup_counts: source_counts,
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

pub fn accept(root: &Path, enrolled_key: &VerifyingKey) -> Result<EvidenceManifestV1, String> {
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
) -> Result<EvidenceManifestV1, String> {
    ensure_real_directory(directory)?;
    let manifest_bytes = read_fixed(directory, MANIFEST_NAME)?;
    let envelope_bytes = read_fixed(directory, SIGNATURE_NAME)?;
    let envelope: SignedEvidenceManifestV1 =
        serde_json::from_slice(&envelope_bytes).map_err(|error| error.to_string())?;
    verify_manifest_signature(&manifest_bytes, &envelope, enrolled_key)?;
    let manifest: EvidenceManifestV1 =
        serde_json::from_slice(&manifest_bytes).map_err(|error| error.to_string())?;
    validate_manifest(&manifest)?;
    let artifact = directory.join(ARTIFACT_NAME);
    let metadata = safe_metadata(&artifact)?;
    if metadata.len() != manifest.byte_length {
        return Err("backup byte length mismatch".into());
    }
    if sha256_file(&artifact)? != manifest.sha256 {
        return Err("backup SHA-256 mismatch".into());
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

fn sign_manifest(bytes: &[u8], key: &SigningKey) -> SignedEvidenceManifestV1 {
    SignedEvidenceManifestV1 {
        version: "SignedEvidenceBackupManifestV1".into(),
        algorithm: "Ed25519".into(),
        signer_key_id: signer_key_id(&key.verifying_key()),
        payload_base64: BASE64.encode(bytes),
        signature_base64: BASE64.encode(key.sign(bytes).to_bytes()),
    }
}

fn verify_manifest_signature(
    literal_manifest: &[u8],
    envelope: &SignedEvidenceManifestV1,
    key: &VerifyingKey,
) -> Result<(), String> {
    if envelope.version != "SignedEvidenceBackupManifestV1"
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

fn validate_manifest(manifest: &EvidenceManifestV1) -> Result<(), String> {
    if manifest.version != "EvidenceBackupManifestV1"
        || manifest.artifact_filename != ARTIFACT_NAME
        || !manifest.evidence_verifier_success
        || !canonical_timestamp(&manifest.created_at)
        || manifest.sha256.len() != 64
        || !manifest
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || manifest.source_counts != manifest.backup_counts
    {
        return Err("invalid evidence manifest".into());
    }
    Ok(())
}

fn parse_counts(report: &str) -> Result<EvidenceCounts, String> {
    let start = report.find('(').ok_or("missing backup counts")? + 1;
    let end = report[start..].find(')').ok_or("missing backup counts")? + start;
    let fields: Vec<&str> = report[start..end].split(", ").collect();
    if fields.len() != 5 {
        return Err("unexpected backup count report".into());
    }
    let number = |index: usize, suffix: &str| -> Result<u64, String> {
        fields[index]
            .strip_suffix(suffix)
            .ok_or_else(|| "unexpected backup count report".to_string())?
            .parse()
            .map_err(|_| "invalid backup count".to_string())
    };
    Ok(EvidenceCounts {
        records: number(1, " records")?,
        observations: number(2, " observations")?,
        edges: number(3, " edges")?,
        schema_version: fields[4]
            .strip_prefix("schema ")
            .ok_or("missing schema count")?
            .parse()
            .map_err(|_| "invalid schema count")?,
    })
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = safe_open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(crate) fn create_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

pub(crate) fn copy_private(source: &Path, destination: &Path) -> Result<(), String> {
    let mut input = safe_open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)
        .map_err(|error| error.to_string())?;
    std::io::copy(&mut input, &mut output).map_err(|error| error.to_string())?;
    output.sync_all().map_err(|error| error.to_string())
}

pub(crate) fn ensure_real_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("{} must be a real directory", path.display()));
    }
    Ok(())
}

pub(crate) fn safe_metadata(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(format!("{} must be a regular file", path.display()));
    }
    Ok(metadata)
}

pub(crate) fn safe_open(path: &Path) -> Result<File, String> {
    let before = safe_metadata(path)?;
    let file = File::open(path).map_err(|error| error.to_string())?;
    let after = file.metadata().map_err(|error| error.to_string())?;
    if before.dev() != after.dev() || before.ino() != after.ino() {
        return Err("artifact changed while opening".into());
    }
    Ok(file)
}

pub(crate) fn read_fixed(directory: &Path, name: &str) -> Result<Vec<u8>, String> {
    let path: PathBuf = directory.join(name);
    let mut file = safe_open(&path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}
