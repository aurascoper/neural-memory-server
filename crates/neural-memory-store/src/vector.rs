//! Vector storage and search. Brute force, deliberately.
//!
//! At this corpus size a full scan of a few thousand 768-float vectors costs a
//! few milliseconds and about 15 MB. An ANN index would add an extension
//! dependency, a build step and approximate results, to optimise something that
//! is not the bottleneck. `tests/vector.rs` measures the scan so the day it
//! stops being fast enough is a measurement rather than a surprise.
//!
//! ## The one property that matters
//!
//! **Vectors from different embedding spaces are never compared.** This is not
//! a performance concern, it is a correctness one: cosine similarity between
//! vectors from two different spaces returns a perfectly plausible number.
//! Nothing errors, nothing looks wrong, and the results are simply incorrect.
//! Carried over from `property_law.rs`: *mixing spaces silently poisons
//! retrieval.*
//!
//! The rule is structural rather than remembered — the space identity is half
//! the primary key and half of every query's `WHERE`, so there is no code path
//! that could compare across spaces even by mistake.

use neural_memory_domain::{
    embedding_space_identity, EmbeddingProfileTerms, Normalization, Pooling,
};
use rusqlite::params;

use crate::{Store, StoreError};

#[derive(Debug)]
pub enum VectorError {
    Sql(rusqlite::Error),
    /// The vector's length does not match the profile's declared dimensions.
    /// Storing it would produce plausible cosines against everything else.
    DimensionMismatch {
        profile: String,
        expected: usize,
        got: usize,
    },
    UnknownProfile(String),
    /// A stored blob is not a whole number of f32s, or not the declared length.
    CorruptVector {
        record: String,
        bytes: usize,
    },
    NotFinite {
        record: String,
    },
}

impl std::fmt::Display for VectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VectorError::Sql(e) => write!(f, "sqlite: {e}"),
            VectorError::DimensionMismatch {
                profile,
                expected,
                got,
            } => write!(
                f,
                "profile {profile} declares {expected} dimensions, got {got}; a vector of the \
                 wrong length would still yield a plausible cosine against every other record"
            ),
            VectorError::UnknownProfile(p) => write!(f, "unknown embedding profile {p}"),
            VectorError::CorruptVector { record, bytes } => {
                write!(
                    f,
                    "vector for {record} is {bytes} bytes, not a whole f32 vector"
                )
            }
            VectorError::NotFinite { record } => {
                write!(f, "vector for {record} contains a non-finite value")
            }
        }
    }
}

impl std::error::Error for VectorError {}

impl From<rusqlite::Error> for VectorError {
    fn from(e: rusqlite::Error) -> Self {
        VectorError::Sql(e)
    }
}
impl From<StoreError> for VectorError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Sql(x) => VectorError::Sql(x),
            StoreError::Migration(m) => VectorError::UnknownProfile(m),
        }
    }
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(b: &[u8], record: &str) -> Result<Vec<f32>, VectorError> {
    if !b.len().is_multiple_of(4) {
        return Err(VectorError::CorruptVector {
            record: record.to_string(),
            bytes: b.len(),
        });
    }
    Ok(b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

/// Cosine similarity, normalising both sides.
///
/// Computed rather than assumed even when the profile declares L2
/// normalisation: a vector that arrived un-normalised despite the declaration
/// would otherwise score arbitrarily high, and the declaration is a claim about
/// the producer, not a guarantee about the bytes.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

#[derive(Clone, Debug)]
pub struct VectorHit {
    pub record_digest: String,
    pub similarity: f32,
}

impl Store {
    /// Register an embedding space. Idempotent; the identity is its seal.
    pub fn register_embedding_profile(
        &self,
        p: &EmbeddingProfileTerms,
        backend: &str,
        seq_len: u32,
        at: &str,
    ) -> Result<String, VectorError> {
        let identity = embedding_space_identity(p);
        let mut w = p.weight_sha256.clone();
        w.sort();
        let mut t = p.tokenizer_sha256.clone();
        t.sort();
        self.conn.execute(
            "INSERT OR IGNORE INTO embedding_profiles
               (id, backend, model_name, dim, seq_len, created_at, identity,
                model_revision, pooling, normalization, task_instruction,
                weight_sha256, tokenizer_sha256)
             VALUES (?1,?2,?3,?4,?5,?6,?1,?7,?8,?9,?10,?11,?12)",
            params![
                identity,
                // Recorded for provenance, NOT part of the identity: whether a
                // CPU-derived and an NPU-derived vector share a space is a
                // question to be measured, not settled by stamping them apart.
                backend,
                p.model_family,
                p.dimensions,
                seq_len,
                at,
                p.model_revision,
                match p.pooling {
                    Pooling::Mean => "mean",
                    Pooling::Cls => "cls",
                    Pooling::LastToken => "lastToken",
                },
                match p.normalization {
                    Normalization::None => "none",
                    Normalization::L2 => "l2",
                },
                p.task_instruction,
                serde_json::to_string(&w).expect("json"),
                serde_json::to_string(&t).expect("json"),
            ],
        )?;
        Ok(identity)
    }

    fn profile_dimensions(&self, profile: &str) -> Result<usize, VectorError> {
        self.conn
            .query_row(
                "SELECT dim FROM embedding_profiles WHERE identity = ?1",
                params![profile],
                |r| r.get::<_, i64>(0),
            )
            .map(|d| d as usize)
            .map_err(|_| VectorError::UnknownProfile(profile.to_string()))
    }

    pub fn put_embedding(
        &self,
        profile: &str,
        record_digest: &str,
        vector: &[f32],
        embedded_text: &str,
        at: &str,
    ) -> Result<(), VectorError> {
        let dim = self.profile_dimensions(profile)?;
        if vector.len() != dim {
            return Err(VectorError::DimensionMismatch {
                profile: profile.to_string(),
                expected: dim,
                got: vector.len(),
            });
        }
        if vector.iter().any(|x| !x.is_finite()) {
            return Err(VectorError::NotFinite {
                record: record_digest.to_string(),
            });
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO embeddings
               (profile_identity, record_digest, vector, embedded_text, embedded_at)
             VALUES (?1,?2,?3,?4,?5)",
            params![profile, record_digest, to_blob(vector), embedded_text, at],
        )?;
        Ok(())
    }

    /// Nearest records **within one embedding space**.
    ///
    /// `profile` is not a filter that could be forgotten — it is the first half
    /// of the primary key, so there is no query shape that reaches across
    /// spaces.
    pub fn vector_search(
        &self,
        profile: &str,
        query: &[f32],
        limit: usize,
        include_retired: bool,
    ) -> Result<Vec<VectorHit>, VectorError> {
        let dim = self.profile_dimensions(profile)?;
        if query.len() != dim {
            return Err(VectorError::DimensionMismatch {
                profile: profile.to_string(),
                expected: dim,
                got: query.len(),
            });
        }

        let sql = if include_retired {
            "SELECT e.record_digest, e.vector FROM embeddings e
             JOIN memories m ON m.record_digest = e.record_digest
             WHERE e.profile_identity = ?1"
        } else {
            "SELECT e.record_digest, e.vector FROM embeddings e
             JOIN memories m ON m.record_digest = e.record_digest
             WHERE e.profile_identity = ?1
               AND m.superseded_at IS NULL AND m.retracted_at IS NULL"
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![profile], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;

        let mut hits = Vec::new();
        for row in rows {
            let (digest, blob) = row?;
            let v = from_blob(&blob, &digest)?;
            if v.len() != dim {
                return Err(VectorError::CorruptVector {
                    record: digest,
                    bytes: blob.len(),
                });
            }
            hits.push(VectorHit {
                record_digest: digest,
                similarity: cosine(query, &v),
            });
        }
        // Ties break on digest so the order is total and reproducible.
        hits.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.record_digest.cmp(&b.record_digest))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    /// How many records carry a vector in this space. Used to report coverage:
    /// a semantic branch over half the corpus is not a semantic branch.
    pub fn embedding_coverage(&self, profile: &str) -> Result<(usize, usize), StoreError> {
        // Joined and filtered the SAME way as the denominator. Counting every
        // embedding against only the live records reports coverage above 100%,
        // which is not a coverage figure -- retired records are embedded too so
        // that `include_retired` search works, and they belong in neither side.
        let embedded: i64 = self.conn.query_row(
            "SELECT count(*) FROM embeddings e
             JOIN memories m ON m.record_digest = e.record_digest
             WHERE e.profile_identity = ?1 AND m.superseded_at IS NULL",
            params![profile],
            |r| r.get(0),
        )?;
        let total: i64 = self.conn.query_row(
            "SELECT count(*) FROM memories WHERE superseded_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        Ok((embedded as usize, total as usize))
    }
}
