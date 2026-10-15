//! Random secrets and how they are stored (only as hashes).

/// Web sessions: `hbs_` + 256 random bits.
pub const SESSION_PREFIX: &str = "hbs_";
/// Device codes polled by the CLI: `hbd_` + 256 random bits.
pub const DEVICE_PREFIX: &str = "hbd_";
/// One-time sign-in links: `hbi_` + 256 random bits.
pub const INVITE_PREFIX: &str = "hbi_";

/// A new secret with `prefix`.
#[must_use]
pub fn token(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    format!("{prefix}{}", hex::encode(bytes))
}

/// How secrets are stored and looked up. They carry 256 random bits, so a
/// fast keyed-less hash is enough (nothing to brute-force).
#[must_use]
pub fn hash(secret: &str) -> String {
    blake3::hash(secret.as_bytes()).to_hex().to_string()
}

/// A code people type: 8 characters from an alphabet without look-alikes,
/// shown as `ABCD-EFGH`.
#[must_use]
pub fn user_code() -> String {
    const ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ23456789";
    let mut bytes = [0u8; 8];
    rand::fill(&mut bytes);
    let chars: String = bytes
        .iter()
        .map(|b| char::from(ALPHABET[usize::from(*b) % ALPHABET.len()]))
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        let t = token(SESSION_PREFIX);
        assert!(t.starts_with("hbs_") && t.len() == 4 + 64);
        assert_ne!(token(SESSION_PREFIX), t);
        let c = user_code();
        assert_eq!(c.len(), 9);
        assert_eq!(&c[4..5], "-");
        assert_eq!(hash("x"), hash("x"));
    }
}
