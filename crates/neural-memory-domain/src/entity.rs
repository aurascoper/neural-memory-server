//! Entity extraction: deterministic, declared, and versioned.
//!
//! Deterministic and effect-free — no model, no network, no clock.
//!
//! ## Why a dictionary rather than a model
//!
//! `mentions.extractor_identity` is `NOT NULL` in the very first migration: a
//! mention has to say *what found it*. That requirement rules out a black-box
//! NER, because "some model, some version, some weights" is not an answer, and
//! a retrieval branch driven by an unrecorded extractor is exactly the
//! unattributed inference this store exists to prevent.
//!
//! So entities are **declared** by the operator and found by exact matching.
//! The trade is deliberate: recall is bounded by what you thought to declare,
//! and in exchange every mention is reproducible, explicable, and attributable
//! to a dictionary whose contents are sealed into the extractor identity. Run
//! the same extractor over the same text and you get the same spans, forever.
//!
//! An LLM extractor could be added later, but its mentions would be
//! `AgentInference`-tier and should not silently drive retrieval alongside
//! these.
//!
//! ## What the branch is actually for
//!
//! Not finding "Gemma" in text that says "Gemma" — full-text search already
//! does that, and better. It is for **aliases**: resolving "the 12B model" or
//! "the green team's runtime" to the entity a record actually names. That is
//! the one thing neither lexical nor semantic reliably does, and it is why the
//! branch earns a weight at all.

use serde::{Deserialize, Serialize};

use crate::digest::seal;

const ENTITY_DOMAIN: &str = "neuralmemory.entity.v1";
const EXTRACTOR_DOMAIN: &str = "neuralmemory.entity-extractor.v1";

/// Bump when the matching ALGORITHM changes in a way that moves spans.
/// It is sealed into the extractor identity, so old mentions remain
/// attributable to the rules that actually produced them.
pub const EXTRACTOR_ALGORITHM: &str = "longest-match-case-insensitive-word-bounded.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityTerms {
    pub canonical_name: String,
    pub entity_type: String,
    /// Surface forms that resolve to this entity. **SORTED** — a set.
    /// The canonical name is always matched too and need not be repeated here.
    pub aliases: Vec<String>,
}

pub fn entity_identity(e: &EntityTerms) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Doc<'a> {
        domain: &'static str,
        canonical_name: &'a str,
        entity_type: &'a str,
        aliases: Vec<String>,
    }
    let mut a = e.aliases.clone();
    a.sort();
    a.dedup();
    seal(&Doc {
        domain: ENTITY_DOMAIN,
        canonical_name: &e.canonical_name,
        entity_type: &e.entity_type,
        aliases: a,
    })
}

/// The dictionary an extraction ran against.
///
/// Its identity is sealed over every entity it contains, so a mention can name
/// not merely "the dictionary extractor" but *which* dictionary. Adding an
/// entity changes the identity, which is correct: the same text would now yield
/// different spans, and pretending otherwise would make old mentions
/// unreproducible.
#[derive(Clone, Debug, Default)]
pub struct EntityDictionary {
    /// `(entity_identity, surface_form_lowercased)`, longest surface first.
    surfaces: Vec<(String, String)>,
    identities: Vec<String>,
}

impl EntityDictionary {
    pub fn new(entities: &[EntityTerms]) -> Self {
        let mut surfaces = Vec::new();
        let mut identities = Vec::new();
        for e in entities {
            let id = entity_identity(e);
            identities.push(id.clone());
            surfaces.push((id.clone(), e.canonical_name.to_lowercase()));
            for a in &e.aliases {
                surfaces.push((id.clone(), a.to_lowercase()));
            }
        }
        // Longest first so "Qwen3 8B Q6_K" wins over "Qwen3 8B" at the same
        // position. Ties broken on the surface text so the order is total and
        // the extractor is reproducible rather than dependent on input order.
        surfaces.sort_by(|a, b| {
            b.1.len()
                .cmp(&a.1.len())
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.0.cmp(&b.0))
        });
        identities.sort();
        identities.dedup();
        Self {
            surfaces,
            identities,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }

    /// Seals the algorithm together with every entity in the dictionary.
    pub fn extractor_identity(&self) -> String {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Doc<'a> {
            domain: &'static str,
            algorithm: &'static str,
            entity_identities: &'a [String],
        }
        seal(&Doc {
            domain: EXTRACTOR_DOMAIN,
            algorithm: EXTRACTOR_ALGORITHM,
            entity_identities: &self.identities,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mention {
    pub entity_identity: String,
    /// Byte offsets into the text as given. Half-open `[start, end)`.
    pub start: usize,
    pub end: usize,
    /// The text actually matched, in its original casing — so a span can be
    /// checked against the source without re-running the extractor.
    pub surface: String,
}

/// Locate `needle` at or after `from`.
///
/// When `offsets_valid` is false the haystack is the ORIGINAL text, because
/// lowercasing changed its byte length and offsets into the lowercased copy
/// would not line up with the source. Matching then becomes case-insensitive
/// only for the ASCII portion, which is the honest trade: correct spans matter
/// more than catching a differently-cased non-ASCII surface form.
fn find_from(hay: &str, needle: &str, from: usize, offsets_valid: bool) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    if offsets_valid {
        hay[from..].find(needle).map(|r| from + r)
    } else {
        let h = hay[from..].to_lowercase();
        // Only usable when this slice's length is stable too.
        if h.len() != hay.len() - from {
            return None;
        }
        h.find(needle).map(|r| from + r)
    }
}

fn is_boundary(bytes: &[u8], at: usize) -> bool {
    if at == 0 || at >= bytes.len() {
        return true;
    }
    !bytes[at].is_ascii_alphanumeric() || !bytes[at - 1].is_ascii_alphanumeric()
}

/// Find every declared entity in `text`.
///
/// Longest match wins and matches never overlap: once a span is taken, a
/// shorter surface inside it is not also reported. Reporting both would make
/// "Qwen3 8B" a hit on every mention of "Qwen3 8B Q6_K" and quietly inflate
/// every entity score.
pub fn extract_mentions(dict: &EntityDictionary, text: &str) -> Vec<Mention> {
    let lower = text.to_lowercase();
    // `to_lowercase` can change byte length for non-ASCII, which would make
    // offsets into `lower` wrong for `text`. Fall back to matching on the
    // original when that happens rather than reporting spans that do not line
    // up with the source.
    let (hay, offsets_valid) = if lower.len() == text.len() {
        (lower.as_str(), true)
    } else {
        (text, false)
    };
    let bytes = hay.as_bytes();

    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut out: Vec<Mention> = Vec::new();

    for (id, surface) in &dict.surfaces {
        if surface.is_empty() {
            continue;
        }
        let mut from = 0usize;
        while let Some(rel) = find_from(hay, surface, from, offsets_valid) {
            let start = rel;
            let end = start + surface.len();
            from = start + 1;
            if !is_boundary(bytes, start) || !is_boundary(bytes, end) {
                continue;
            }
            if taken.iter().any(|(s, e)| start < *e && end > *s) {
                continue;
            }
            if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                continue;
            }
            if text[start..end].to_lowercase() != *surface {
                continue;
            }
            taken.push((start, end));
            out.push(Mention {
                entity_identity: id.clone(),
                start,
                end,
                surface: text[start..end].to_string(),
            });
        }
    }
    out.sort_by_key(|m| (m.start, m.end));
    out
}
