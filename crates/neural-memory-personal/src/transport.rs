use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::runtime::{
    load_enrolled_verifying_key, load_or_create_signing_key, parse_cursor, store_now,
};
use crate::{sign_payload, signer_key_id, verify_envelope, PersonalStore, SyncEnvelopeV1};

pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

pub fn require_loopback(address: SocketAddr) -> Result<SocketAddr, String> {
    if address.ip().is_loopback() {
        Ok(address)
    } else {
        Err("listen address must be loopback".into())
    }
}

pub struct TransportConfig {
    pub database: PathBuf,
    pub signing_key: PathBuf,
    pub peer_key: PathBuf,
    pub source_device: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", deny_unknown_fields)]
enum Request {
    #[serde(rename = "status")]
    Status,
    #[serde(rename = "publicKey")]
    PublicKey,
    #[serde(rename = "export")]
    Export { after: String },
    #[serde(rename = "acknowledge")]
    Acknowledge { through: String },
    #[serde(rename = "import")]
    Import { envelope: SyncEnvelopeV1 },
}

#[derive(Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub body: Value,
}

impl Response {
    fn error(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            body: json!({"error":{"code":code,"message":message.into()}}),
        }
    }
}

pub fn authorize(header: Option<&str>, token: &[u8]) -> bool {
    let Some(candidate) = header.and_then(|value| value.strip_prefix("Bearer ")) else {
        return false;
    };
    constant_time_equal(candidate.as_bytes(), token)
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let length = left.len().max(right.len());
    for index in 0..length {
        difference |= usize::from(
            left.get(index).copied().unwrap_or(0) ^ right.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

pub fn handle(
    config: &TransportConfig,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    token: &[u8],
    body: &[u8],
) -> Response {
    if method != "POST" {
        return Response::error(405, "methodNotAllowed", "POST is required");
    }
    if path != "/v1/personal-sync" {
        return Response::error(404, "notFound", "unknown path");
    }
    if !authorize(authorization, token) {
        return Response::error(401, "unauthorized", "invalid bearer credential");
    }
    if body.len() > MAX_BODY_BYTES {
        return Response::error(413, "bodyTooLarge", "request exceeds 8 MiB");
    }
    let value: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(error) => return Response::error(400, "invalidRequest", error.to_string()),
    };
    if let Err(message) = validate_shape(&value) {
        return Response::error(400, "invalidRequest", message);
    }
    let request: Request = match serde_json::from_value(value) {
        Ok(request) => request,
        Err(error) => return Response::error(400, "invalidRequest", error.to_string()),
    };
    match execute(config, request) {
        Ok(body) => Response { status: 200, body },
        Err(message) => Response::error(400, "operationRejected", message),
    }
}

fn validate_shape(value: &Value) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "request must be an object".to_string())?;
    let action = object
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "action must be a string".to_string())?;
    let allowed: &[&str] = match action {
        "status" => &["action"],
        "publicKey" => &["action"],
        "export" => &["action", "after"],
        "acknowledge" => &["action", "through"],
        "import" => &["action", "envelope"],
        _ => return Err("unknown action".into()),
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("unknown field".into());
    }
    Ok(())
}

fn execute(config: &TransportConfig, request: Request) -> Result<Value, String> {
    match request {
        Request::Status => Ok(json!({"health":"ready"})),
        Request::PublicKey => {
            let key = load_or_create_signing_key(&config.signing_key)?;
            let public = key.verifying_key();
            Ok(
                json!({"algorithm":"Ed25519","signerKeyID":signer_key_id(&public),"publicKeyBase64":BASE64.encode(public.as_bytes())}),
            )
        }
        Request::Export { after } => {
            let after = parse_cursor(&after)?;
            let key = load_or_create_signing_key(&config.signing_key)?;
            let mut store = PersonalStore::open(Path::new(&config.database))
                .map_err(|error| error.to_string())?;
            let generated_at = store_now(&store)?;
            let payload = store
                .export_promotions(&config.source_device, after, &generated_at)
                .map_err(|error| error.to_string())?;
            serde_json::to_value(sign_payload(&payload, &key).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())
        }
        Request::Acknowledge { through } => {
            let through = parse_cursor(&through)?;
            let store = PersonalStore::open(Path::new(&config.database))
                .map_err(|error| error.to_string())?;
            let acknowledged_at = store_now(&store)?;
            let promoted = store
                .acknowledge_promotions(through, &acknowledged_at)
                .map_err(|error| error.to_string())?;
            Ok(json!({"acknowledgedThrough":through,"promoted":promoted}))
        }
        Request::Import { envelope } => {
            let trusted = load_enrolled_verifying_key(&config.peer_key)?;
            let payload =
                verify_envelope(&envelope, &trusted).map_err(|error| error.to_string())?;
            let mut store = PersonalStore::open(Path::new(&config.database))
                .map_err(|error| error.to_string())?;
            let committed = store
                .import_verified_bundle(&envelope, &trusted)
                .map_err(|error| error.to_string())?;
            Ok(json!({"ack":{"through":payload.through,"committed":committed}}))
        }
    }
}
