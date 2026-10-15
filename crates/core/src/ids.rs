//! Identifiers: content hashes for objects and stable random ids for changes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// BLAKE3 hash of a stored object (blob, tree or commit).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ObjectId([u8; 32]);

impl ObjectId {
    /// Hashes `data` as an object of the given kind.
    #[must_use]
    pub fn hash(kind: &str, data: &[u8]) -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(kind.as_bytes());
        hasher.update(&[0]);
        hasher.update(data);
        Self(*hasher.finalize().as_bytes())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Short form used in human output.
    #[must_use]
    pub fn short(&self) -> String {
        let mut hex = self.hex();
        hex.truncate(12);
        hex
    }

    pub fn parse(s: &str) -> Result<Self> {
        let bytes = hex::decode(s).map_err(|_| Error::InvalidId(s.to_owned()))?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| Error::InvalidId(s.to_owned()))?;
        Ok(Self(bytes))
    }
}

impl TryFrom<String> for ObjectId {
    type Error = Error;
    fn try_from(s: String) -> Result<Self> {
        Self::parse(&s)
    }
}

impl From<ObjectId> for String {
    fn from(id: ObjectId) -> Self {
        id.hex()
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({})", self.short())
    }
}

/// Stable identity of a change. It survives rewrites (amend, describe, rebase),
/// unlike the commit id. Rendered with the letters `k`-`z` so it can never be
/// confused with a hex commit id.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ChangeId([u8; 16]);

const CHANGE_ALPHABET_TOP: u8 = b'z';

impl ChangeId {
    #[must_use]
    pub fn random() -> Self {
        let mut bytes = [0u8; 16];
        rand::fill(&mut bytes);
        Self(bytes)
    }

    /// The change id of the root commit.
    #[must_use]
    pub const fn root() -> Self {
        Self([0; 16])
    }

    #[must_use]
    pub fn encode(&self) -> String {
        let mut out = String::with_capacity(32);
        for byte in self.0 {
            out.push(char::from(CHANGE_ALPHABET_TOP - (byte >> 4)));
            out.push(char::from(CHANGE_ALPHABET_TOP - (byte & 0x0f)));
        }
        out
    }

    /// Short form used in human output.
    #[must_use]
    pub fn short(&self) -> String {
        let mut s = self.encode();
        s.truncate(8);
        s
    }

    pub fn parse(s: &str) -> Result<Self> {
        let raw = s.as_bytes();
        if raw.len() != 32 || !is_change_id_prefix(s) {
            return Err(Error::InvalidId(s.to_owned()));
        }
        let mut bytes = [0u8; 16];
        for (i, pair) in raw.chunks_exact(2).enumerate() {
            bytes[i] = ((CHANGE_ALPHABET_TOP - pair[0]) << 4) | (CHANGE_ALPHABET_TOP - pair[1]);
        }
        Ok(Self(bytes))
    }
}

/// Whether `s` only uses the change id alphabet (`k`-`z`).
#[must_use]
pub fn is_change_id_prefix(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| (b'k'..=b'z').contains(&b))
}

impl TryFrom<String> for ChangeId {
    type Error = Error;
    fn try_from(s: String) -> Result<Self> {
        Self::parse(&s)
    }
}

impl From<ChangeId> for String {
    fn from(id: ChangeId) -> Self {
        id.encode()
    }
}

impl fmt::Display for ChangeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.encode())
    }
}

impl fmt::Debug for ChangeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChangeId({})", self.short())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_id_round_trips() {
        let id = ChangeId::random();
        assert_eq!(ChangeId::parse(&id.encode()).unwrap(), id);
        assert!(is_change_id_prefix(&id.encode()));
    }

    #[test]
    fn object_id_round_trips() {
        let id = ObjectId::hash("blob", b"hello");
        assert_eq!(ObjectId::parse(&id.hex()).unwrap(), id);
        assert_ne!(id, ObjectId::hash("tree", b"hello"));
    }
}
