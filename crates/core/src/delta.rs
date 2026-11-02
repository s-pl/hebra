//! Binary deltas: an object as a list of "copy from the base" and "insert
//! these bytes" instructions. Consecutive versions of a file or directory
//! differ by little, so this is what keeps a repository small.
//!
//! Layout: the target length as a varint, then instructions:
//! `0, len, bytes` inserts and `1, offset, len` copies (numbers as varints).

const WINDOW: usize = 8;
/// A shorter match costs more to describe than to insert.
const MIN_COPY: usize = 12;
const INSERT: u8 = 0;
const COPY: u8 = 1;

fn put_varint(out: &mut Vec<u8>, mut n: usize) {
    while n >= 0x80 {
        out.push((n & 0x7f) as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn get_varint(data: &[u8], at: &mut usize) -> Option<usize> {
    let mut value = 0usize;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(*at)?;
        *at += 1;
        value |= usize::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= usize::BITS {
            return None;
        }
    }
}

fn hash(window: &[u8], bits: u32) -> usize {
    let mut bytes = [0u8; WINDOW];
    bytes.copy_from_slice(&window[..WINDOW]);
    let mixed = u64::from_le_bytes(bytes).wrapping_mul(0x9E37_79B1_85EB_CA87);
    usize::try_from(mixed >> (64 - bits)).unwrap_or(0)
}

fn flush_insert(out: &mut Vec<u8>, pending: &[u8]) {
    if !pending.is_empty() {
        out.push(INSERT);
        put_varint(out, pending.len());
        out.extend_from_slice(pending);
    }
}

/// Offsets remembered for each hash. Files repeat themselves (imports, blank
/// lines, indentation), and the first match is often not the longest.
const WAYS: usize = 4;

/// How far `base[from..]` and `target[at..]` agree.
fn common(base: &[u8], from: usize, target: &[u8], at: usize) -> usize {
    base[from..]
        .iter()
        .zip(&target[at..])
        .take_while(|(a, b)| a == b)
        .count()
}

/// `target` as a delta against `base`, or `None` when they share too little
/// for a delta to be worth it.
#[must_use]
pub fn encode(base: &[u8], target: &[u8]) -> Option<Vec<u8>> {
    if base.len() < MIN_COPY || target.len() < MIN_COPY {
        return None;
    }
    let bits = (base.len().ilog2() + 1).clamp(10, 24);
    let mut table = vec![[0u32; WAYS]; 1 << bits];
    let last = u32::try_from(base.len().checked_sub(WINDOW)?).ok()?;
    let mut pos = 0u32;
    while pos <= last {
        let at = pos as usize;
        let bucket = &mut table[hash(&base[at..], bits)];
        bucket.rotate_right(1);
        bucket[0] = pos + 1;
        pos += 4;
    }

    let mut out = Vec::with_capacity(target.len() / 4 + 16);
    put_varint(&mut out, target.len());
    let mut pending_from = 0;
    // Where the last copy ended: after an edit, the text usually goes on
    // from there.
    let mut resume: Option<usize> = Some(0);
    let mut i = 0;
    while i + WINDOW <= target.len() {
        let mut best: Option<(usize, usize)> = None;
        let consider = |from: usize, best: &mut Option<(usize, usize)>| {
            let len = common(base, from, target, i);
            if len >= WINDOW && best.is_none_or(|(_, l)| len > l) {
                *best = Some((from, len));
            }
        };
        if let Some(from) = resume.filter(|f| *f < base.len()) {
            consider(from, &mut best);
        }
        for &candidate in &table[hash(&target[i..], bits)] {
            if candidate != 0 {
                consider((candidate - 1) as usize, &mut best);
            }
        }
        let Some((mut from, mut len)) = best else {
            i += 1;
            continue;
        };
        // Grow backwards over bytes that would otherwise be inserted.
        let mut start = i;
        while from > 0 && start > pending_from && base[from - 1] == target[start - 1] {
            from -= 1;
            start -= 1;
            len += 1;
        }
        if len < MIN_COPY {
            i += 1;
            continue;
        }
        flush_insert(&mut out, &target[pending_from..start]);
        out.push(COPY);
        put_varint(&mut out, from);
        put_varint(&mut out, len);
        i = start + len;
        pending_from = i;
        // A short match is often a coincidence: keep following the text.
        if len >= 32 {
            resume = Some(from + len);
        }
        if out.len() >= target.len() {
            return None;
        }
    }
    flush_insert(&mut out, &target[pending_from..]);
    (out.len() < target.len()).then_some(out)
}

/// Rebuilds the target. `None` means the delta is corrupt.
#[must_use]
pub fn apply(base: &[u8], delta: &[u8]) -> Option<Vec<u8>> {
    let mut at = 0;
    let size = get_varint(delta, &mut at)?;
    // The size comes from the file: never trust it for an allocation.
    let mut out = Vec::with_capacity(size.min(delta.len().saturating_mul(64)));
    while at < delta.len() {
        let op = delta[at];
        at += 1;
        match op {
            INSERT => {
                let len = get_varint(delta, &mut at)?;
                let end = at.checked_add(len)?;
                out.extend_from_slice(delta.get(at..end)?);
                at = end;
            }
            COPY => {
                let from = get_varint(delta, &mut at)?;
                let len = get_varint(delta, &mut at)?;
                out.extend_from_slice(base.get(from..from.checked_add(len)?)?);
            }
            _ => return None,
        }
        if out.len() > size {
            return None;
        }
    }
    (out.len() == size).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize, marker: &str) -> Vec<u8> {
        let mut out = String::new();
        for i in 0..n {
            out.push_str(&format!("line number {i} of a file {marker}\n"));
        }
        out.into_bytes()
    }

    #[test]
    fn round_trips_and_is_small_for_small_edits() {
        let base = lines(2000, "");
        let mut target = base.clone();
        target.splice(
            5000..5000,
            b"a new line right in the middle\n".iter().copied(),
        );
        target.extend_from_slice(b"and one at the end\n");
        let delta = encode(&base, &target).unwrap();
        assert!(delta.len() < 200, "delta is {} bytes", delta.len());
        assert_eq!(apply(&base, &delta).unwrap(), target);
    }

    #[test]
    fn refuses_unrelated_or_tiny_inputs() {
        assert!(encode(b"short", b"short too").is_none());
        let a: Vec<u8> = (0..4096u32).map(|i| (i * 7919 % 251) as u8).collect();
        let b: Vec<u8> = (0..4096u32).map(|i| (i * 104_729 % 241) as u8).collect();
        assert!(encode(&a, &b).is_none());
    }

    #[test]
    fn corrupt_deltas_fail_cleanly() {
        let base = lines(100, "");
        let mut target = base.clone();
        target.extend_from_slice(b"more");
        let delta = encode(&base, &target).unwrap();
        for cut in 0..delta.len() {
            assert_ne!(apply(&base, &delta[..cut]).as_deref(), Some(&target[..]));
        }
        assert!(apply(&base, &[5, 9, 9, 9]).is_none());
        // A huge declared size must not allocate that much.
        let mut huge = Vec::new();
        put_varint(&mut huge, usize::MAX / 2);
        assert!(apply(&base, &huge).is_none());
    }

    #[test]
    fn handles_prefix_suffix_and_empty_changes() {
        let base = lines(300, "x");
        for target in [
            base[100..].to_vec(),
            base[..base.len() - 100].to_vec(),
            base.clone(),
        ] {
            let delta = encode(&base, &target).unwrap();
            assert_eq!(apply(&base, &delta).unwrap(), target);
        }
    }
}
