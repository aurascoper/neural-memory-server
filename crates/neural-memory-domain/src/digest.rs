//! Content hashing and the domain-separation recipe.
//!
//! Deterministic and effect-free: this module opens no file, reads no clock,
//! and performs no I/O.
//!
//! The recipe is carried over verbatim from `neuralcompose-mobile-core`
//! (`conformance.rs:116,140`, `runtime_target.rs:215,234,255`), which applies it
//! at nine call sites:
//!
//! 1. a `const DOMAIN: &str = "<system>.<kebab-thing>.v<N>"`,
//! 2. a serializable document with `domain` as the **first** field,
//! 3. every SORTED list sorted before serialization,
//! 4. `sha256_hex(serde_json::to_vec(&doc))`.
//!
//! Two deliberate departures from upstream:
//!
//! - **Namespace.** Upstream owns `neuralcompose.*`. This crate mints under
//!   `neuralmemory.*`. Two systems minting under one prefix would reintroduce
//!   exactly the collision domain separation exists to prevent.
//! - **Placement.** Upstream's `sha256_hex` lives in `audio.rs`, and
//!   `valid_sha256` is duplicated in `runtime_target.rs` and `model_pack.rs`.
//!   Both are accidents worth not copying; they live here, once.
//!
//! ## A known limitation, stated rather than discovered later
//!
//! "Canonical JSON" here means *serde_json's default rendering of a struct whose
//! lists have been sorted* — struct field order is the canonical order. There is
//! no JCS/CBOR canonicalization. Any `f64` in a digested document therefore goes
//! through serde_json's float formatter, so cross-language digest agreement is
//! **not** guaranteed for float-bearing documents. Where a float must be sealed,
//! carry it as a decimal string instead.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// Lowercase hex SHA-256 of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Is this a well-formed 64-hex lowercase digest?
///
/// Uppercase is rejected deliberately: two spellings of one digest would be two
/// distinct primary keys, which is a silent duplication rather than an error.
pub fn valid_sha256(s: &str) -> bool {
    s.len() == 64
        && s.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Seal a domain-separated document. The caller supplies a struct whose first
/// field is `domain`; this only serializes and hashes it.
pub(crate) fn seal<T: serde::Serialize>(doc: &T) -> String {
    sha256_hex(&serde_json::to_vec(doc).expect("digest document must serialize"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn valid_sha256_rejects_uppercase_and_wrong_length() {
        let ok = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(valid_sha256(ok));
        // Polarity: the same digest in uppercase is NOT the same key.
        assert!(!valid_sha256(&ok.to_uppercase()));
        assert!(!valid_sha256(&ok[..63]));
        assert!(!valid_sha256(&format!("{ok}0")));
        assert!(!valid_sha256(""));
        assert!(!valid_sha256(&"g".repeat(64)));
    }
}
