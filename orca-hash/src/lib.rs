//! Hashing primitives shared across orca crates.
//!
//! Provides the [`Hash`](type@Hash) identifier type (SHA-256, 32 bytes), the
//! [`Hasher`]/[`Hashable`] traits used to stream commit contents into a
//! digest, the [`Sha256Hasher`] implementation, and serde helpers to
//! (de)serialize hashes as lowercase hex strings.
//!
//! `std::hash` is deliberately not used: its `finish() -> u64` output is too
//! small for an identifier and its byte representation is not stable across
//! releases.

#![warn(missing_docs)]

use sha2::Digest;

/// 32-byte SHA-256 digest used as the commit / layer identifier.
///
/// A `Hash` is computed once when a commit is built and never recomputed
/// afterwards (it is an immutable ID, not a Merkle link).
pub type Hash = [u8; 32];

/// Error returned when parsing a hex string into a [`Hash`](type@Hash).
#[derive(Debug, thiserror::Error)]
pub enum HashParseError {
    /// The input was not valid hexadecimal.
    #[error("invalid hex string: {0}")]
    InvalidHex(#[from] hex::FromHexError),
    /// The input decoded to a length other than 32 bytes.
    #[error("invalid hash length: expected 32 bytes, got {0}")]
    InvalidLength(usize),
}

/// Render a [`Hash`](type@Hash) as a lowercase hex string (64 chars).
pub fn to_hex(hash: &Hash) -> String {
    hex::encode(hash)
}

/// Parse a lowercase/uppercase hex string into a [`Hash`](type@Hash).
///
/// Returns [`HashParseError`] if the string is not valid hex or does not
/// decode to exactly 32 bytes.
pub fn from_hex(s: &str) -> Result<Hash, HashParseError> {
    let bytes = hex::decode(s)?;
    let len = bytes.len();
    bytes
        .try_into()
        .map_err(|_| HashParseError::InvalidLength(len))
}

/// Byte sink for hashing. Analogous to `std::hash::Hasher`, but backed by a
/// cryptographic digest whose `finalize()` yields a 32-byte [`Hash`](type@Hash).
pub trait Hasher {
    /// Absorb raw bytes into the digest.
    fn update(&mut self, bytes: &[u8]);

    /// Absorb a length-prefixed (framed) field.
    ///
    /// Framing prevents ambiguity collisions between adjacent
    /// variable-length fields (`"ab" + "c"` vs `"a" + "bc"`).
    fn update_framed(&mut self, bytes: &[u8]) {
        self.update(&(bytes.len() as u64).to_le_bytes());
        self.update(bytes);
    }
}

/// A value that can stream itself into a [`Hasher`].
///
/// Modeled after `std::hash::Hash` but defined locally so the digest is
/// stable. Implementations write directly into the hasher without building
/// intermediate buffers. The trait is object-safe (`&dyn Hashable`) so that
/// `CommitBuilder` can hold a reference to either an `Upper` or a `Layer`.
pub trait Hashable {
    /// Stream this value into `hasher`.
    fn hash(&self, hasher: &mut dyn Hasher);
}

/// A [`Hash`](type@Hash) hashes as its raw 32 bytes (fixed length, so no framing).
impl Hashable for Hash {
    fn hash(&self, hasher: &mut dyn Hasher) {
        hasher.update(self);
    }
}

/// SHA-256 backed [`Hasher`].
pub struct Sha256Hasher(sha2::Sha256);

impl Sha256Hasher {
    /// Create a fresh hasher.
    pub fn new() -> Self {
        Self(sha2::Sha256::new())
    }

    /// Consume the hasher and return the 32-byte digest.
    pub fn finalize(self) -> Hash {
        self.0.finalize().into()
    }
}

impl Default for Sha256Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher for Sha256Hasher {
    fn update(&mut self, bytes: &[u8]) {
        Digest::update(&mut self.0, bytes);
    }
}

/// Serde helpers serializing a [`Hash`](type@Hash) as a lowercase hex string.
///
/// Use with `#[serde(with = "orca_hash::hash_serde")]`. Submodules
/// [`hash_serde::vec`] and [`hash_serde::option`] cover `Vec<Hash>` and
/// `Option<Hash>` fields.
pub mod hash_serde {
    use super::{Hash, from_hex, to_hex};
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serialize a [`Hash`](type@Hash) as a hex string.
    pub fn serialize<S: Serializer>(hash: &Hash, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&to_hex(hash))
    }

    /// Deserialize a [`Hash`](type@Hash) from a hex string.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Hash, D::Error> {
        let s = String::deserialize(d)?;
        from_hex(&s).map_err(serde::de::Error::custom)
    }

    /// Serde helpers for `Vec<Hash>` fields.
    pub mod vec {
        use super::{Hash, from_hex, to_hex};
        use serde::{Deserialize, Deserializer, Serializer};

        /// Serialize a `Vec<Hash>` as a list of hex strings.
        pub fn serialize<S: Serializer>(hashes: &[Hash], s: S) -> Result<S::Ok, S::Error> {
            s.collect_seq(hashes.iter().map(to_hex))
        }

        /// Deserialize a `Vec<Hash>` from a list of hex strings.
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Hash>, D::Error> {
            let strings = Vec::<String>::deserialize(d)?;
            strings
                .iter()
                .map(|s| from_hex(s).map_err(serde::de::Error::custom))
                .collect()
        }
    }

    /// Serde helpers for `Option<Hash>` fields.
    ///
    /// TOML has no `null`, so pair this with
    /// `#[serde(default, skip_serializing_if = "Option::is_none")]`.
    pub mod option {
        use super::{Hash, from_hex, to_hex};
        use serde::{Deserialize, Deserializer, Serializer};

        /// Serialize an `Option<Hash>` as an optional hex string.
        pub fn serialize<S: Serializer>(hash: &Option<Hash>, s: S) -> Result<S::Ok, S::Error> {
            match hash {
                Some(h) => s.serialize_some(&to_hex(h)),
                None => s.serialize_none(),
            }
        }

        /// Deserialize an `Option<Hash>` from an optional hex string.
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Hash>, D::Error> {
            let s = Option::<String>::deserialize(d)?;
            s.map(|s| from_hex(&s).map_err(serde::de::Error::custom))
                .transpose()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_value() {
        let mut h = Sha256Hasher::new();
        h.update(b"abc");
        assert_eq!(
            to_hex(&h.finalize()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn framing_prevents_boundary_collision() {
        let mut a = Sha256Hasher::new();
        a.update_framed(b"ab");
        a.update_framed(b"c");
        let mut b = Sha256Hasher::new();
        b.update_framed(b"a");
        b.update_framed(b"bc");
        assert_ne!(a.finalize(), b.finalize());
    }

    #[test]
    fn hex_roundtrip() {
        let hash: Hash = [0xab; 32];
        assert_eq!(from_hex(&to_hex(&hash)).unwrap(), hash);
        assert!(matches!(
            from_hex("abcd"),
            Err(HashParseError::InvalidLength(2))
        ));
        assert!(from_hex("zz").is_err());
    }

    #[test]
    fn hash_is_hashable_without_framing() {
        let hash: Hash = [1; 32];
        let mut h = Sha256Hasher::new();
        Hashable::hash(&hash, &mut h);
        let mut expected = Sha256Hasher::new();
        expected.update(&[1; 32]);
        assert_eq!(h.finalize(), expected.finalize());
    }
}
