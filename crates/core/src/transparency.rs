//! The transparency log's hash chain, shared by the server (which appends)
//! and clients (which verify with `hb verify-log`).

/// The `prev_hash` of a repository's first entry.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The hash of an entry: BLAKE3 over the previous hash and the entry's
/// fields, each followed by a newline. `payload` is compact JSON with keys
/// in sorted order.
#[must_use]
pub fn entry_hash(prev_hash: &str, seq: i64, at_ms: i64, kind: &str, payload: &str) -> String {
    let mut h = blake3::Hasher::new();
    for part in [
        prev_hash,
        &seq.to_string(),
        &at_ms.to_string(),
        kind,
        payload,
    ] {
        h.update(part.as_bytes());
        h.update(b"\n");
    }
    h.finalize().to_hex().to_string()
}

/// What the server signs for a repository's head.
#[must_use]
pub fn head_statement(repo: &str, seq: i64, hash: &str) -> String {
    format!("{repo}\n{seq}\n{hash}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_chain() {
        let a = entry_hash(GENESIS, 1, 10, "bookmark", "{}");
        let b = entry_hash(&a, 2, 11, "bookmark", "{}");
        assert_ne!(a, b);
        let a2 = entry_hash(GENESIS, 1, 10, "bookmark", "{\"x\":1}");
        assert_ne!(entry_hash(&a2, 2, 11, "bookmark", "{}"), b);
    }
}
