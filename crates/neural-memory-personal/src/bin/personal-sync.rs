use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use neural_memory_personal::runtime::{
    decode_verifying_key, enroll_peer_verifying_key, load_enrolled_verifying_key,
    load_existing_signing_key, load_or_create_signing_key, parse_cursor, rotate_signing_key,
    store_now,
};
use neural_memory_personal::*;
use serde_json::json;

fn required_env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

fn exact_flag<'a>(args: &'a [String], expected: &str) -> Result<&'a str, String> {
    if args.len() != 2 || args[0] != expected {
        return Err(format!("expected {expected} VALUE"));
    }
    Ok(&args[1])
}

fn run() -> Result<serde_json::Value, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = args.split_first().ok_or("subcommand required")?;
    let key_path = PathBuf::from(required_env("NEURAL_MEMORY_PERSONAL_KEY")?);
    match command.as_str() {
        "public-key" if rest.is_empty() => {
            let key = load_or_create_signing_key(&key_path)?;
            let public = key.verifying_key();
            Ok(
                json!({"algorithm":"Ed25519","signerKeyID":signer_key_id(&public),"publicKeyBase64":BASE64.encode(public.as_bytes())}),
            )
        }
        "export" => {
            let after = parse_cursor(exact_flag(rest, "--after")?)?;
            let db = PathBuf::from(required_env("NEURAL_MEMORY_PERSONAL_DB")?);
            let source = required_env("NEURAL_MEMORY_PERSONAL_DEVICE")?;
            let key = load_or_create_signing_key(&key_path)?;
            let mut store = PersonalStore::open(&db).map_err(|e| e.to_string())?;
            let at = store_now(&store)?;
            let payload = store
                .export_promotions(&source, after, &at)
                .map_err(|e| e.to_string())?;
            serde_json::to_value(sign_payload(&payload, &key).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())
        }
        "acknowledge" => {
            let through = parse_cursor(exact_flag(rest, "--through")?)?;
            let db = PathBuf::from(required_env("NEURAL_MEMORY_PERSONAL_DB")?);
            let store = PersonalStore::open(&db).map_err(|e| e.to_string())?;
            let at = store_now(&store)?;
            let promoted = store
                .acknowledge_promotions(through, &at)
                .map_err(|e| e.to_string())?;
            Ok(json!({"acknowledgedThrough":through,"promoted":promoted}))
        }
        "import" if rest.is_empty() => {
            let peer_key = PathBuf::from(required_env("NEURAL_MEMORY_MAC_PUBLIC_KEY")?);
            let trusted = load_enrolled_verifying_key(&peer_key)?;
            let db = PathBuf::from(required_env("NEURAL_MEMORY_PERSONAL_DB")?);
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .map_err(|e| e.to_string())?;
            let envelope: SyncEnvelopeV1 =
                serde_json::from_str(&input).map_err(|e| format!("envelope JSON: {e}"))?;
            let payload = verify_envelope(&envelope, &trusted).map_err(|e| e.to_string())?;
            let mut store = PersonalStore::open(&db).map_err(|e| e.to_string())?;
            let committed = store
                .import_verified_bundle(&envelope, &trusted)
                .map_err(|e| e.to_string())?;
            Ok(json!({"ack":{"through":payload.through,"committed":committed}}))
        }
        "enroll-peer" => {
            if rest.len() != 4
                || rest[0] != "--public-key-base64"
                || rest[2] != "--confirm"
                || rest[3] != "ENROLL-MAC-PEER"
            {
                return Err("expected --public-key-base64 KEY --confirm ENROLL-MAC-PEER".into());
            }
            let peer_key = PathBuf::from(required_env("NEURAL_MEMORY_MAC_PUBLIC_KEY")?);
            let key = decode_verifying_key(&rest[1])?;
            enroll_peer_verifying_key(&peer_key, &key)?;
            Ok(json!({"algorithm":"Ed25519","enrolledSignerKeyID":signer_key_id(&key)}))
        }
        "reset-replica" => {
            let epoch: u64 = exact_flag(rest, "--expected-predecessor-epoch")?
                .parse()
                .map_err(|_| "invalid predecessor epoch")?;
            load_existing_signing_key(&key_path)?;
            let db = PathBuf::from(required_env("NEURAL_MEMORY_PERSONAL_DB")?);
            let mut store = PersonalStore::open(&db).map_err(|e| e.to_string())?;
            let removed = store
                .reset_replica_for_reenrollment(epoch)
                .map_err(|e| e.to_string())?;
            Ok(
                json!({"replicaReset":{"expectedPredecessorEpoch":epoch,"removedReplicaOnlyRecords":removed}}),
            )
        }
        "rotate-key" => {
            if exact_flag(rest, "--confirm")? != "ROTATE-SIGNING-KEY" {
                return Err("rotation confirmation required".into());
            }
            let rotated = rotate_signing_key(&key_path)?;
            Ok(json!({"oldSignerKeyID":rotated.old_key_id,"newSignerKeyID":rotated.new_key_id}))
        }
        "bump-epoch-and-snapshot" => {
            if rest.len() != 4
                || rest[0] != "--at"
                || rest[2] != "--confirm"
                || rest[3] != "BUMP-EPOCH-AND-SNAPSHOT"
            {
                return Err("expected --at TIMESTAMP --confirm BUMP-EPOCH-AND-SNAPSHOT".into());
            }
            load_existing_signing_key(&key_path)?;
            let db = PathBuf::from(required_env("NEURAL_MEMORY_PERSONAL_DB")?);
            let mut store = PersonalStore::open(&db).map_err(|e| e.to_string())?;
            let through = store
                .bump_promotion_epoch_and_snapshot(&rest[1])
                .map_err(|e| e.to_string())?;
            Ok(json!({"snapshotThrough":through}))
        }
        _ => Err("unknown or malformed subcommand".into()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
