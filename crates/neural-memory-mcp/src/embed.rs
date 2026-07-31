//! Query and document embedding over llama.cpp's OpenAI-compatible endpoint.
//!
//! Network I/O lives here rather than in the store, which does SQLite and
//! nothing else. That division is why retrieval degrades cleanly: if no embedder
//! is configured or the service is down, the semantic branch is simply absent
//! and lexical + provenance still answer. The caller is told the branch was
//! missing rather than handed quietly worse results.
//!
//! ## Document and query prefixes
//!
//! nomic-embed uses asymmetric prefixes -- `search_document:` for what is
//! indexed, `search_query:` for what is asked. Only the **document** prefix is
//! part of the embedding-space identity, because the space is defined by how the
//! corpus was embedded. Querying with the wrong prefix gives worse results
//! against the same space; it does not create a different one. The query prefix
//! is therefore launch configuration, recorded with the run rather than sealed
//! into the space.

use serde_json::json;

pub struct Embedder {
    pub url: String,
    pub profile_identity: String,
    pub document_prefix: String,
    pub query_prefix: String,
    pub dimensions: usize,
}

#[derive(Debug)]
pub enum EmbedError {
    Transport(String),
    Shape { expected: usize, got: usize },
    Malformed(String),
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmbedError::Transport(e) => write!(f, "embedding service unreachable: {e}"),
            EmbedError::Shape { expected, got } => write!(
                f,
                "embedder returned {got} dimensions, profile declares {expected}; storing that \
                 would put vectors of two shapes in one space"
            ),
            EmbedError::Malformed(m) => write!(f, "malformed embedding response: {m}"),
        }
    }
}

impl Embedder {
    fn embed_raw(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let resp = ureq::post(&format!("{}/v1/embeddings", self.url))
            .timeout(std::time::Duration::from_secs(120))
            .send_json(json!({"input": text, "model": "embed"}))
            .map_err(|e| EmbedError::Transport(e.to_string()))?;
        let body: serde_json::Value = resp
            .into_json()
            .map_err(|e| EmbedError::Malformed(e.to_string()))?;
        let arr = body["data"][0]["embedding"]
            .as_array()
            .ok_or_else(|| EmbedError::Malformed("no data[0].embedding".into()))?;
        let v: Vec<f32> = arr
            .iter()
            .filter_map(|x| x.as_f64())
            .map(|x| x as f32)
            .collect();
        if v.len() != self.dimensions {
            return Err(EmbedError::Shape {
                expected: self.dimensions,
                got: v.len(),
            });
        }
        if v.iter().any(|x| !x.is_finite()) {
            return Err(EmbedError::Malformed("non-finite component".into()));
        }
        Ok(v)
    }

    /// Embed a record for indexing. Returns the vector and the exact text sent,
    /// because the prefix is part of what the model saw and reconstructing it
    /// later would be guessing.
    pub fn embed_document(&self, text: &str) -> Result<(Vec<f32>, String), EmbedError> {
        let full = format!("{}{}", self.document_prefix, text);
        Ok((self.embed_raw(&full)?, full))
    }

    pub fn embed_query(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        self.embed_raw(&format!("{}{}", self.query_prefix, text))
    }

    /// Is the service actually answering? Checked at launch so a dead embedder
    /// is a startup diagnostic rather than a silent absence of results.
    pub fn probe(&self) -> Result<(), EmbedError> {
        self.embed_raw("probe").map(|_| ())
    }
}
