use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use neural_memory_personal::interaction_logs_dr::{
    accept, read_current, stage, InteractionLogsManifestV1, SignedInteractionLogsManifestV1,
    StageConfig, ARTIFACT_NAME, MANIFEST_NAME, SIGNATURE_NAME,
};
use neural_memory_personal::runtime::{decode_verifying_key, load_or_create_signing_key};
use serde_json::json;

fn env_path(name: &str) -> Result<PathBuf, String> {
    std::env::var(name)
        .map(PathBuf::from)
        .map_err(|_| format!("{name} is required"))
}

fn root() -> Result<PathBuf, String> {
    let root = env_path("NEURAL_MEMORY_INTERACTION_LOGS_DR_DIR")?;
    if root != Path::new("/srv/neural-memory-data/backups/interaction-logs-dr") {
        return Err(
            "DR directory must be /srv/neural-memory-data/backups/interaction-logs-dr".into(),
        );
    }
    Ok(root)
}

fn exact_value<'a>(args: &'a [String], flag: &str) -> Result<&'a str, String> {
    if args.len() != 2 || args[0] != flag {
        return Err(format!("expected {flag} VALUE"));
    }
    Ok(&args[1])
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = args.split_first().ok_or("command required")?;
    let root = root()?;
    match command.as_str() {
        "stage" => {
            if rest.len() != 3 || rest[0] != "--writers-stopped" || rest[1] != "--created-at" {
                return Err("expected stage --writers-stopped --created-at TIMESTAMP".into());
            }
            if std::env::var("NEURAL_MEMORY_INTERACTION_LOGS_WRITERS_STOPPED").as_deref() != Ok("1")
            {
                return Err("writer-stop environment gate is not set".into());
            }
            let source = env_path("NEURAL_MEMORY_INTERACTION_LOGS_SOURCE")?;
            if source != Path::new("/srv/neural-memory-data/interaction-logs") {
                return Err(
                    "interaction-logs source must be /srv/neural-memory-data/interaction-logs"
                        .into(),
                );
            }
            let tar = env_path("NEURAL_MEMORY_INTERACTION_LOGS_TAR")?;
            if tar != Path::new("/usr/bin/tar") {
                return Err("tar must be /usr/bin/tar".into());
            }
            let key_path = env_path("NEURAL_MEMORY_PERSONAL_KEY")?;
            if key_path != Path::new("/srv/neural-memory-data/keys/gpd-ed25519.seed") {
                return Err(
                    "signing key must be /srv/neural-memory-data/keys/gpd-ed25519.seed".into(),
                );
            }
            let key = load_or_create_signing_key(&key_path)?;
            let manifest = stage(&StageConfig {
                root: &root,
                source: &source,
                tar: &tar,
                created_at: &rest[2],
                signing_key: &key,
            })?;
            println!(
                "{}",
                serde_json::to_string(&manifest).map_err(|error| error.to_string())?
            );
        }
        "accept" => {
            let trusted = decode_verifying_key(exact_value(rest, "--trusted-key-base64")?)?;
            let manifest = accept(&root, &trusted)?;
            println!(
                "{}",
                serde_json::to_string(&json!({"accepted":manifest}))
                    .map_err(|error| error.to_string())?
            );
        }
        "list" if rest.is_empty() => {
            let manifest: InteractionLogsManifestV1 =
                serde_json::from_slice(&read_current(&root, MANIFEST_NAME)?)
                    .map_err(|error| error.to_string())?;
            let signature: SignedInteractionLogsManifestV1 =
                serde_json::from_slice(&read_current(&root, SIGNATURE_NAME)?)
                    .map_err(|error| error.to_string())?;
            println!(
                "{}",
                serde_json::to_string(&json!({"manifest":manifest,"signature":signature}))
                    .map_err(|error| error.to_string())?
            );
        }
        "stream" => {
            if rest.len() != 1 {
                return Err("expected stream backup|manifest|signature".into());
            }
            let name = match rest[0].as_str() {
                "backup" => ARTIFACT_NAME,
                "manifest" => MANIFEST_NAME,
                "signature" => SIGNATURE_NAME,
                _ => return Err("artifact must be backup, manifest, or signature".into()),
            };
            std::io::stdout()
                .write_all(&read_current(&root, name)?)
                .map_err(|error| error.to_string())?;
        }
        _ => return Err("unknown or malformed DR command".into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("interaction-logs DR command rejected: {error}");
            ExitCode::from(2)
        }
    }
}
