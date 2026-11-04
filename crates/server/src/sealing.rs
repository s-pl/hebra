//! Sealing secrets at rest (CI secrets, registry credentials): XChaCha20-
//! Poly1305 with a key derived from the server's mandate signing key, so a
//! copy of the database alone reveals nothing. Each value is bound to the
//! place it belongs to (`scope/owner/name`), so a sealed value cannot be
//! moved to another secret.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::error::{ApiError, ApiResult};

const NONCE: usize = 24;

pub struct Sealer {
    cipher: XChaCha20Poly1305,
}

impl Sealer {
    /// From the mandate key's hex (the one secret every server has).
    #[must_use]
    pub fn from_signing_key(hex: &str) -> Self {
        let key = blake3::derive_key(
            "hebra-server 2026-11 secrets at rest",
            hex.trim().as_bytes(),
        );
        Self {
            cipher: XChaCha20Poly1305::new(&key.into()),
        }
    }

    pub fn seal(&self, place: &str, value: &[u8]) -> ApiResult<String> {
        let mut nonce = [0u8; NONCE];
        rand::fill(&mut nonce);
        let sealed = self
            .cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: value,
                    aad: place.as_bytes(),
                },
            )
            .map_err(|_| ApiError::internal("sealing", "encryption failed"))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&sealed);
        Ok(hex::encode(out))
    }

    pub fn open(&self, place: &str, sealed: &str) -> ApiResult<Vec<u8>> {
        let bytes = hex::decode(sealed).map_err(|e| ApiError::internal("sealing", e))?;
        if bytes.len() < NONCE {
            return Err(ApiError::internal("sealing", "sealed value too short"));
        }
        let (nonce, body) = bytes.split_at(NONCE);
        let nonce: [u8; NONCE] = nonce
            .try_into()
            .map_err(|_| ApiError::internal("sealing", "nonce"))?;
        self.cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: body,
                    aad: place.as_bytes(),
                },
            )
            .map_err(|_| {
                ApiError::internal("sealing", "a sealed value does not open (key changed?)")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_values_open_only_in_their_place() {
        let s = Sealer::from_signing_key("00ff");
        let sealed = s.seal("repo/r1/TOKEN", b"hunter2").unwrap();
        assert_eq!(s.open("repo/r1/TOKEN", &sealed).unwrap(), b"hunter2");
        assert!(s.open("repo/r2/TOKEN", &sealed).is_err());
        assert!(
            Sealer::from_signing_key("other")
                .open("repo/r1/TOKEN", &sealed)
                .is_err()
        );
    }
}
