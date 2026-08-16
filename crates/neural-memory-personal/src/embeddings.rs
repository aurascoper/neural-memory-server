use std::path::Path;

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::{Host, Url};

use crate::{canonical_timestamp, PersonalError, PersonalStore};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmbeddingProfile {
    pub backend: String,
    pub model_artifact: String,
    pub dimension: u32,
    pub normalization: String,
    pub version: String,
    pub adapter: String,
    pub endpoint: Option<String>,
}

impl EmbeddingProfile {
    pub fn identity(&self) -> Result<String, PersonalError> {
        let bytes = serde_json_canonicalizer::to_vec(self)
            .map_err(|error| PersonalError::Metadata(error.to_string()))?;
        Ok(Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    fn validate(&self) -> Result<(), PersonalError> {
        if self.backend.trim().is_empty()
            || self.model_artifact.trim().is_empty()
            || self.version.trim().is_empty()
            || self.dimension == 0
            || self.dimension > 65_536
            || !matches!(self.normalization.as_str(), "l2" | "none")
            || !matches!(
                self.adapter.as_str(),
                "llama-cpp-http" | "deterministic-test"
            )
            || (self.adapter == "llama-cpp-http"
                && self.endpoint.as_deref().is_none_or(str::is_empty))
            || (self.adapter == "deterministic-test" && self.endpoint.is_some())
        {
            return Err(PersonalError::Metadata("invalid embedding profile".into()));
        }
        if self.adapter == "llama-cpp-http" {
            validate_endpoint(self.endpoint.as_deref().expect("checked above"))?;
        }
        Ok(())
    }
}

fn validate_endpoint(endpoint: &str) -> Result<(), PersonalError> {
    let url = Url::parse(endpoint)
        .map_err(|_| PersonalError::Metadata("invalid production embedding endpoint".into()))?;
    let port = url.port().ok_or_else(|| {
        PersonalError::Metadata("embedding endpoint requires an explicit port".into())
    })?;
    if port == 0 {
        return Err(PersonalError::Metadata(
            "embedding endpoint port must be between 1 and 65535".into(),
        ));
    }
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(PersonalError::Metadata(
            "embedding endpoint must be a root loopback HTTP URL without credentials, query, or fragment".into(),
        ));
    }
    let canonical = match url.host() {
        Some(Host::Ipv4(address)) if address == std::net::Ipv4Addr::LOCALHOST => {
            format!("http://127.0.0.1:{port}")
        }
        Some(Host::Ipv6(address)) if address == std::net::Ipv6Addr::LOCALHOST => {
            format!("http://[::1]:{port}")
        }
        Some(Host::Domain("localhost")) => format!("http://localhost:{port}"),
        _ => {
            return Err(PersonalError::Metadata(
                "embedding endpoint host must be exactly 127.0.0.1, ::1, or localhost".into(),
            ))
        }
    };
    if endpoint != canonical {
        return Err(PersonalError::Metadata(
            "embedding endpoint must use an unambiguous canonical form".into(),
        ));
    }
    Ok(())
}

pub trait PersonalEmbedder {
    fn profile(&self) -> &EmbeddingProfile;
    fn probe(&self) -> Result<(), String>;
    fn embed_document(&self, text: &str) -> Result<Vec<f32>, String>;
}

pub struct DeterministicTestEmbedder {
    profile: EmbeddingProfile,
}

impl DeterministicTestEmbedder {
    pub fn new(dimension: u32) -> Self {
        Self {
            profile: EmbeddingProfile {
                backend: "deterministic-test-only".into(),
                model_artifact: "not-a-production-model".into(),
                dimension,
                normalization: "l2".into(),
                version: "test-v1".into(),
                adapter: "deterministic-test".into(),
                endpoint: None,
            },
        }
    }
}

impl PersonalEmbedder for DeterministicTestEmbedder {
    fn profile(&self) -> &EmbeddingProfile {
        &self.profile
    }
    fn probe(&self) -> Result<(), String> {
        self.profile.validate().map_err(|error| error.to_string())
    }
    fn embed_document(&self, text: &str) -> Result<Vec<f32>, String> {
        let dimension = self.profile.dimension as usize;
        let mut vector = Vec::with_capacity(dimension);
        for index in 0..dimension {
            let mut hasher = Sha256::new();
            hasher.update(b"personal-embedding-test-v1\0");
            hasher.update((index as u64).to_le_bytes());
            hasher.update(text.as_bytes());
            let bytes = hasher.finalize();
            let raw = i32::from_le_bytes(bytes[..4].try_into().expect("four bytes"));
            vector.push(raw as f32 / i32::MAX as f32);
        }
        normalize(&mut vector);
        Ok(vector)
    }
}

pub struct LlamaCppEmbedder {
    profile: EmbeddingProfile,
}

impl LlamaCppEmbedder {
    pub fn new(profile: EmbeddingProfile) -> Result<Self, String> {
        profile.validate().map_err(|error| error.to_string())?;
        if profile.adapter != "llama-cpp-http" {
            return Err("production adapter requires llama-cpp-http profile".into());
        }
        Ok(Self { profile })
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let endpoint = self
            .profile
            .endpoint
            .as_deref()
            .ok_or("embedding endpoint unavailable")?;
        let response = ureq::post(&format!("{endpoint}/v1/embeddings"))
            .timeout(std::time::Duration::from_secs(120))
            .send_json(json!({"input":text,"model":"embed"}));
        let response = match response {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => {
                let body = response.into_string().unwrap_or_default();
                if terminal_input_rejection(&body) {
                    return Err("record-rejected:input-too-large".into());
                }
                return Err("embedding model rejected request".into());
            }
            Err(error) => return Err(format!("embedding model unavailable: {error}")),
        };
        let body: Value = response.into_json().map_err(|error| error.to_string())?;
        let values = body["data"][0]["embedding"]
            .as_array()
            .ok_or("malformed embedding response")?;
        let mut vector = values
            .iter()
            .map(|value| {
                value
                    .as_f64()
                    .map(|number| number as f32)
                    .ok_or("non-number vector component")
            })
            .collect::<Result<Vec<_>, _>>()?;
        if vector.len() != self.profile.dimension as usize
            || vector.iter().any(|value| !value.is_finite())
        {
            return Err("embedding shape or values do not match profile".into());
        }
        if self.profile.normalization == "l2" {
            normalize(&mut vector);
        }
        Ok(vector)
    }
}

fn terminal_input_rejection(body: &str) -> bool {
    body.contains("input")
        && (body.contains("too large") || body.contains("larger than the max context size"))
}

#[cfg(test)]
mod tests {
    use super::terminal_input_rejection;

    #[test]
    fn only_observed_input_limit_phrasings_are_terminal() {
        assert!(terminal_input_rejection(
            "input (602 tokens) is too large to process"
        ));
        assert!(terminal_input_rejection(
            "input (2070 tokens) is larger than the max context size (2048 tokens). skipping"
        ));
        assert!(!terminal_input_rejection("internal server error"));
        assert!(!terminal_input_rejection("context unavailable"));
        assert!(!terminal_input_rejection("request is too large"));
    }
}

impl PersonalEmbedder for LlamaCppEmbedder {
    fn profile(&self) -> &EmbeddingProfile {
        &self.profile
    }
    fn probe(&self) -> Result<(), String> {
        self.embed("probe").map(|_| ())
    }
    fn embed_document(&self, text: &str) -> Result<Vec<f32>, String> {
        self.embed(text)
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingStatus {
    pub database_path: String,
    pub profile_identity: Option<String>,
    pub local_active: u64,
    pub replica_active: u64,
    pub pending: u64,
    pub ready: u64,
    pub stale: u64,
    pub promotion_epoch: u64,
    pub promotion_sequence: u64,
    pub promotion_pending: u64,
    pub replica_epoch: u64,
    pub replica_sequence: u64,
    pub replicated_as_of: Option<String>,
    pub unacknowledged_divergences: u64,
    pub health: String,
}

impl PersonalStore {
    pub(crate) fn semantic_branch(&self, digest: &str) -> Result<Value, PersonalError> {
        let active: Option<String> = self
            .conn
            .query_row(
                "SELECT profile_identity FROM personal_active_embedding_profile WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(profile_identity) = active else {
            return Ok(json!({"ran":false,"reason":"no-active-profile"}));
        };
        let ready: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM personal_embeddings WHERE profile_identity=?1 AND record_digest=?2)",
            params![profile_identity, digest],
            |row| row.get(0),
        )?;
        if ready {
            return Ok(json!({"ran":true,"profileIdentity":profile_identity}));
        }
        let failure: Option<String> = self
            .conn
            .query_row(
                "SELECT reason FROM personal_embedding_failures WHERE profile_identity=?1 AND record_digest=?2",
                params![profile_identity, digest],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(reason) = failure {
            return Ok(json!({"ran":false,"reason":reason,"profileIdentity":profile_identity}));
        }
        let pending: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM personal_embedding_queue WHERE profile_identity=?1 AND record_digest=?2)",
            params![profile_identity, digest],
            |row| row.get(0),
        )?;
        if pending {
            return Ok(
                json!({"ran":false,"reason":"pending-local-embedding","profileIdentity":profile_identity}),
            );
        }
        Ok(
            json!({"ran":false,"reason":"pending-local-embedding","profileIdentity":profile_identity}),
        )
    }

    pub fn set_embedding_profile(
        &mut self,
        profile: &EmbeddingProfile,
        at: &str,
    ) -> Result<String, PersonalError> {
        if !canonical_timestamp(at) {
            return Err(PersonalError::Metadata(
                "noncanonical profile activation time".into(),
            ));
        }
        profile.validate()?;
        let identity = profile.identity()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT profile_identity FROM personal_active_embedding_profile WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        tx.execute(
            "INSERT OR IGNORE INTO personal_embedding_profiles(identity,backend,model_artifact,dimension,normalization,version,adapter,endpoint) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![identity, profile.backend, profile.model_artifact, profile.dimension, profile.normalization, profile.version, profile.adapter, profile.endpoint],
        )?;
        let stored_adapter: String = tx.query_row(
            "SELECT adapter FROM personal_embedding_profiles WHERE identity=?1",
            [&identity],
            |row| row.get(0),
        )?;
        if stored_adapter != profile.adapter {
            return Err(PersonalError::Conflict(
                "profile identity is already registered with another adapter".into(),
            ));
        }
        tx.execute("INSERT INTO personal_active_embedding_profile(singleton,profile_identity) VALUES (1,?1) ON CONFLICT(singleton) DO UPDATE SET profile_identity=excluded.profile_identity", [&identity])?;
        if previous.as_deref() != Some(identity.as_str()) {
            tx.execute(
                "INSERT INTO personal_embedding_queue(record_digest,profile_identity,enqueued_at)
                 SELECT digest,?1,?2 FROM canonical_records WHERE tombstoned=0
                 ON CONFLICT(record_digest) DO UPDATE SET profile_identity=excluded.profile_identity,enqueued_at=excluded.enqueued_at",
                params![identity, at],
            )?;
        }
        tx.commit()?;
        Ok(identity)
    }

    pub fn rebuild_embeddings(
        &mut self,
        embedder: &dyn PersonalEmbedder,
        limit: usize,
        at: &str,
    ) -> Result<usize, PersonalError> {
        if limit == 0 || limit > 10_000 || !canonical_timestamp(at) {
            return Err(PersonalError::Metadata(
                "invalid rebuild bound or timestamp".into(),
            ));
        }
        embedder.probe().map_err(PersonalError::Metadata)?;
        let expected = embedder.profile().identity()?;
        let active: String = self
            .conn
            .query_row(
                "SELECT profile_identity FROM personal_active_embedding_profile WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(PersonalError::Sql)?;
        if expected != active {
            return Err(PersonalError::Conflict(
                "embedder profile does not match active profile".into(),
            ));
        }
        let active_adapter: String = self.conn.query_row(
            "SELECT adapter FROM personal_embedding_profiles WHERE identity=?1",
            [&active],
            |row| row.get(0),
        )?;
        if active_adapter != embedder.profile().adapter {
            return Err(PersonalError::Conflict(
                "embedder adapter does not match active profile configuration".into(),
            ));
        }
        let todo = {
            let mut statement = self.conn.prepare("SELECT q.record_digest,r.content FROM personal_embedding_queue q JOIN canonical_records r ON r.digest=q.record_digest WHERE q.profile_identity=?1 AND r.tombstoned=0 ORDER BY r.created_at,r.digest LIMIT ?2")?;
            let rows = statement
                .query_map(params![active, limit as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let mut completed = 0;
        for (digest, text) in todo {
            let vector = match embedder.embed_document(&text) {
                Ok(vector) => vector,
                Err(error) if error == "record-rejected:input-too-large" => {
                    let tx = self
                        .conn
                        .transaction_with_behavior(TransactionBehavior::Immediate)?;
                    tx.execute(
                        "INSERT OR REPLACE INTO personal_embedding_failures(profile_identity,record_digest,reason,failed_at) VALUES (?1,?2,'input-too-large',?3)",
                        params![active, digest, at],
                    )?;
                    tx.execute(
                        "DELETE FROM personal_embedding_queue WHERE record_digest=?1 AND profile_identity=?2",
                        params![digest, active],
                    )?;
                    tx.commit()?;
                    continue;
                }
                Err(error) => return Err(PersonalError::Metadata(error)),
            };
            if vector.len() != embedder.profile().dimension as usize
                || vector.iter().any(|value| !value.is_finite())
            {
                return Err(PersonalError::Metadata("invalid embedding vector".into()));
            }
            let blob: Vec<u8> = vector
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute("INSERT OR REPLACE INTO personal_embeddings(profile_identity,record_digest,vector,embedded_at) VALUES (?1,?2,?3,?4)", params![active,digest,blob,at])?;
            tx.execute("DELETE FROM personal_embedding_failures WHERE record_digest=?1 AND profile_identity=?2", params![digest,active])?;
            tx.execute("DELETE FROM personal_embedding_queue WHERE record_digest=?1 AND profile_identity=?2", params![digest,active])?;
            tx.commit()?;
            completed += 1;
        }
        Ok(completed)
    }

    pub fn embedding_status(&self, database_path: &Path) -> Result<EmbeddingStatus, PersonalError> {
        let profile: Option<String> = self
            .conn
            .query_row(
                "SELECT profile_identity FROM personal_active_embedding_profile WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let count = |sql: &str| -> Result<u64, PersonalError> {
            Ok(self.conn.query_row(sql, [], |row| row.get::<_, i64>(0))? as u64)
        };
        let (promotion_epoch, promotion_sequence): (i64, i64) = self.conn.query_row(
            "SELECT epoch,sequence FROM promotion_state WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let cursor = self.cursor()?;
        let integrity: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        let unacknowledged_divergences =
            count("SELECT count(*) FROM personal_divergences WHERE status='unacknowledged'")?;
        let health = if integrity != "ok" {
            "failed".into()
        } else if unacknowledged_divergences != 0 {
            "blocked:unacknowledged-divergence".into()
        } else if profile.is_none() {
            "degraded:no-active-profile".into()
        } else {
            "ok".into()
        };
        Ok(EmbeddingStatus {
            database_path: database_path.display().to_string(), profile_identity: profile,
            local_active: count("SELECT count(DISTINCT c.record_digest) FROM captures c JOIN canonical_records r ON r.digest=c.record_digest WHERE r.tombstoned=0")?,
            replica_active: count("SELECT count(*) FROM replica_records x JOIN canonical_records r ON r.digest=x.record_digest WHERE r.tombstoned=0")?,
            pending: count("SELECT count(*) FROM personal_embedding_queue q JOIN canonical_records r ON r.digest=q.record_digest WHERE r.tombstoned=0")?,
            ready: count("SELECT count(*) FROM personal_embeddings e JOIN personal_active_embedding_profile a ON a.profile_identity=e.profile_identity JOIN canonical_records r ON r.digest=e.record_digest WHERE r.tombstoned=0")?,
            stale: count("SELECT (SELECT count(*) FROM personal_embeddings e JOIN canonical_records r ON r.digest=e.record_digest WHERE r.tombstoned=0 AND NOT EXISTS (SELECT 1 FROM personal_active_embedding_profile a WHERE a.profile_identity=e.profile_identity)) + (SELECT count(*) FROM personal_embedding_failures f JOIN canonical_records r ON r.digest=f.record_digest JOIN personal_active_embedding_profile a ON a.profile_identity=f.profile_identity WHERE r.tombstoned=0)")?,
            promotion_epoch:promotion_epoch as u64,promotion_sequence:promotion_sequence as u64,
            promotion_pending:count("SELECT count(*) FROM promotion_outbox WHERE status='pending'")?,replica_epoch:cursor.epoch,replica_sequence:cursor.sequence,replicated_as_of:cursor.replicated_as_of,
            unacknowledged_divergences,
            health,
        })
    }

    pub fn local_context(&self, query: &str, limit: usize) -> Result<Value, PersonalError> {
        if query.trim().is_empty() || limit == 0 || limit > 100 {
            return Err(PersonalError::Metadata(
                "invalid context query or bound".into(),
            ));
        }
        let pattern = format!("%{query}%");
        let mut statement=self.conn.prepare("SELECT digest,content,created_at,occurred_at FROM canonical_records r WHERE tombstoned=0 AND NOT EXISTS (SELECT 1 FROM personal_divergences d WHERE d.status='unacknowledged' AND (d.digest_a=r.digest OR d.digest_b=r.digest)) AND content LIKE ?1 ORDER BY created_at DESC,digest LIMIT ?2")?;
        let rows = statement
            .query_map(params![pattern, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let rows = rows
            .into_iter()
            .map(|(digest, text, created_at, occurred_at)| {
                Ok(json!({"contentDigest":digest,"text":text,"createdAt":created_at,"occurredAt":occurred_at,"semanticBranch":self.semantic_branch(&digest)?}))
            })
            .collect::<Result<Vec<_>, PersonalError>>()?;
        Ok(json!({"query":query,"records":rows}))
    }
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector {
            *value /= norm;
        }
    }
}
