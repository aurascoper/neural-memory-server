use rusqlite::params;
use serde_json::{json, Value};

use crate::{Capture, PersonalError, PersonalStore, Sighting};

pub const PROTOCOL_VERSION: &str = "2025-06-18";

fn definitions() -> Value {
    json!([
        {"name":"remember","description":"Bank a personal memory locally.","inputSchema":{"type":"object","properties":{"text":{"type":"string","minLength":1},"occurredAt":{"type":["string","null"]},"metadata":{"type":"object"},"createdAt":{"type":"string"},"source":{"type":"string"},"conversationID":{"type":"string"},"originDevice":{"type":"string","minLength":1},"originRecordID":{"type":"string","minLength":1},"tags":{"type":"array","items":{"type":"string"},"uniqueItems":true}},"required":["text","createdAt","originDevice","originRecordID"],"additionalProperties":false}},
        {"name":"recall","description":"Search active personal memories by text and optional tag.","inputSchema":{"type":"object","properties":{"query":{"type":"string","minLength":1},"tag":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"required":["query"],"additionalProperties":false}},
        {"name":"list_recent","description":"List recently captured active personal memories.","inputSchema":{"type":"object","properties":{"tag":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}},"additionalProperties":false}},
        {"name":"forget","description":"Tombstone one personal memory locally.","inputSchema":{"type":"object","properties":{"contentDigest":{"type":"string","pattern":"^[0-9a-f]{64}$"},"forgottenAt":{"type":"string"}},"required":["contentDigest","forgottenAt"],"additionalProperties":false}}
    ])
}

fn nonempty<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("'{key}' is required and must be non-empty"))
}

fn digest(args: &Value) -> Result<&str, String> {
    let value = nonempty(args, "contentDigest")?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("'contentDigest' must be 64 lowercase hexadecimal characters".into());
    }
    Ok(value)
}

fn rows(
    store: &PersonalStore,
    query: Option<&str>,
    tag: Option<&str>,
    limit: u64,
) -> Result<Value, PersonalError> {
    let pattern = query.map(|value| format!("%{value}%"));
    let mut statement = store.conn.prepare(
        "SELECT r.digest, r.content, r.occurred_at, r.created_at, r.metadata
           FROM canonical_records r
          WHERE r.tombstoned = 0
            AND NOT EXISTS (
                SELECT 1 FROM personal_divergences d
                 WHERE d.status = 'unacknowledged'
                   AND (d.digest_a = r.digest OR d.digest_b = r.digest))
            AND (?1 IS NULL OR r.content LIKE ?1)
            AND (?2 IS NULL OR EXISTS (
                SELECT 1 FROM record_tags rt WHERE rt.record_digest = r.digest AND rt.tag = ?2))
          ORDER BY r.created_at DESC, r.digest LIMIT ?3",
    )?;
    let records = statement
        .query_map(params![pattern, tag, limit.min(100) as i64], |row| {
            let digest: String = row.get(0)?;
            let metadata: String = row.get(4)?;
            Ok((
                digest,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                metadata,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut output = Vec::with_capacity(records.len());
    for (digest, text, occurred_at, created_at, metadata) in records {
        let mut tags = store
            .conn
            .prepare("SELECT tag FROM record_tags WHERE record_digest = ?1 ORDER BY tag")?;
        let tags = tags
            .query_map([&digest], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut sightings = store.conn.prepare("SELECT origin_device, origin_id, created_at, source, conversation FROM sightings WHERE record_digest = ?1 ORDER BY created_at, origin_device, origin_id")?;
        let sightings = sightings.query_map([&digest], |row| Ok(json!({"originDevice":row.get::<_,String>(0)?,"originRecordID":row.get::<_,String>(1)?,"capturedAt":row.get::<_,String>(2)?,"source":row.get::<_,Option<String>>(3)?,"conversationID":row.get::<_,Option<String>>(4)?})))?.collect::<Result<Vec<_>, _>>()?;
        let semantic_branch = store.semantic_branch(&digest)?;
        output.push(json!({"contentDigest":digest,"text":text,"occurredAt":occurred_at,"metadata":serde_json::from_str::<Value>(&metadata).expect("validated JSON"),"createdAt":created_at,"tags":tags,"sightings":sightings,"semanticBranch":semantic_branch}));
    }
    Ok(Value::Array(output))
}

pub fn call_tool(store: &mut PersonalStore, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "remember" => {
            let text = nonempty(args, "text")?;
            let created_at = nonempty(args, "createdAt")?;
            let origin_device = nonempty(args, "originDevice")?;
            let origin_id = nonempty(args, "originRecordID")?;
            let metadata = args.get("metadata").cloned().unwrap_or_else(|| json!({}));
            if !metadata.is_object() {
                return Err("'metadata' must be an object".into());
            }
            let metadata_json = serde_json::to_string(&metadata).expect("value serializes");
            let tags = match args.get("tags") {
                None => Vec::new(),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| "'tags' entries must be strings".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                Some(_) => return Err("'tags' must be an array".into()),
            };
            let capture = Capture {
                content: text,
                occurred_at: args.get("occurredAt").and_then(Value::as_str),
                metadata_json: &metadata_json,
                sighting: Sighting {
                    created_at: created_at.into(),
                    source: args
                        .get("source")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    conversation: args
                        .get("conversationID")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    origin_device: origin_device.into(),
                    origin_id: origin_id.into(),
                },
                tags,
            };
            let digest = store.capture(&capture).map_err(|e| e.to_string())?;
            Ok(json!({"contentDigest":digest,"banked":true}))
        }
        "recall" => rows(
            store,
            Some(nonempty(args, "query")?),
            args.get("tag").and_then(Value::as_str),
            args.get("limit").and_then(Value::as_u64).unwrap_or(20),
        )
        .map_err(|e| e.to_string()),
        "list_recent" => rows(
            store,
            None,
            args.get("tag").and_then(Value::as_str),
            args.get("limit").and_then(Value::as_u64).unwrap_or(20),
        )
        .map_err(|e| e.to_string()),
        "forget" => Ok(
            json!({"forgotten":store.forget(digest(args)?, nonempty(args,"forgottenAt")?).map_err(|e| e.to_string())?}),
        ),
        _ => Err(format!("unknown personal tool: {name}")),
    }
}

pub fn handle_request(store: &mut PersonalStore, request: &Value) -> Option<Value> {
    request.get("id")?;
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let result = match request.get("method").and_then(Value::as_str) {
        Some("initialize") => Ok(
            json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{"tools":{}},"serverInfo":{"name":"neural-memory-personal","version":env!("CARGO_PKG_VERSION")}}),
        ),
        Some("tools/list") => Ok(json!({"tools":definitions()})),
        Some("tools/call") => {
            let params = request.get("params").unwrap_or(&Value::Null);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").unwrap_or(&Value::Null);
            call_tool(store, name, args).map(|value| json!({"content":[{"type":"text","text":serde_json::to_string(&value).expect("serialize")}]}))
        }
        Some(method) => {
            return Some(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("method not found: {method}")}}),
            )
        }
        None => Err("missing method".into()),
    };
    Some(match result {
        Ok(value) => json!({"jsonrpc":"2.0","id":id,"result":value}),
        Err(message) => {
            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":message}],"isError":true}})
        }
    })
}
