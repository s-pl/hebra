//! `hb verify-log`: checks a repository's transparency log on your machine.
//!
//! 1. every entry chains to the previous one (its hash covers it);
//! 2. the server's Ed25519 signature over the head is valid (and, with
//!    `--key`, made by the key you pinned);
//! 3. replaying the bookmark moves gives the bookmarks the server shows now.
//!
//! A server that rewrote history would fail one of the three.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Result, bail};
use hebra_core::Workspace;
use hebra_core::transparency::{GENESIS, entry_hash, head_statement};
use serde_json::{Value, json};

use crate::commands::Output;
use crate::remote::Remote;

pub fn verify(ws: &Workspace, remote: &str, pinned_key: Option<&str>) -> Result<Output> {
    let r = Remote::load(ws, remote, Duration::from_secs(60))?;
    let mut entries: Vec<Value> = Vec::new();
    let mut page = r.get_json("/transparency")?;
    let repo = page["repo"].as_str().unwrap_or_default().to_owned();
    let public_key = page["public_key"].as_str().unwrap_or_default().to_owned();
    let head = page["head"].clone();
    loop {
        let batch = page["entries"].as_array().cloned().unwrap_or_default();
        let last = batch.last().and_then(|e| e["seq"].as_i64());
        entries.extend(batch);
        match last {
            Some(seq) if entries.len().is_multiple_of(1000) => {
                page = r.get_json(&format!("/transparency?after={seq}"))?
            }
            _ => break,
        }
    }

    // 1. The chain.
    let mut prev = GENESIS.to_owned();
    let mut bookmarks: BTreeMap<String, String> = BTreeMap::new();
    for (i, e) in entries.iter().enumerate() {
        let seq = e["seq"].as_i64().unwrap_or_default();
        if seq != i64::try_from(i).unwrap_or(0) + 1 {
            bail!("entry {seq} is out of sequence (expected {})", i + 1);
        }
        if e["prev_hash"].as_str() != Some(prev.as_str()) {
            bail!("entry {seq} does not chain to the one before it");
        }
        let at_ms = e["at_ms"].as_i64().unwrap_or_default();
        let payload = e["payload"].to_string();
        let kind = e["kind"].as_str().unwrap_or("");
        let expected = entry_hash(&prev, seq, at_ms, kind, &payload);
        if e["hash"].as_str() != Some(expected.as_str()) {
            bail!("entry {seq} was altered (its hash does not match its contents)");
        }
        if kind == "bookmark" {
            let name = e["payload"]["name"].as_str().unwrap_or_default().to_owned();
            match e["payload"]["new"].as_str() {
                Some(target) => {
                    bookmarks.insert(name, target.to_owned());
                }
                None => {
                    bookmarks.remove(&name);
                }
            }
        }
        prev = expected;
    }

    // 2. The signed head.
    let head_seq = head["seq"].as_i64().unwrap_or(0);
    let head_hash = head["hash"].as_str().unwrap_or("");
    if !entries.is_empty() {
        if head_seq != i64::try_from(entries.len()).unwrap_or(0) || head_hash != prev {
            bail!("the signed head does not match the log");
        }
        let signature = head["signature"].as_str().unwrap_or("");
        if !hebra_mandate::verify_signature(
            &public_key,
            head_statement(&repo, head_seq, head_hash).as_bytes(),
            signature,
        ) {
            bail!("the head's signature is invalid");
        }
    }
    if let Some(pinned) = pinned_key
        && pinned != public_key
    {
        bail!("the log is signed by {public_key}, not the key you pinned ({pinned})");
    }

    // 3. The bookmarks the server shows now.
    let info = r.get_json("")?;
    let current: BTreeMap<String, String> = info["bookmarks"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    if current != bookmarks {
        let differ: Vec<&String> = current
            .keys()
            .chain(bookmarks.keys())
            .filter(|k| current.get(*k) != bookmarks.get(*k))
            .collect();
        bail!("bookmarks differ from what the log records: {differ:?}");
    }

    Ok(Output::new(
        json!({ "repo": repo, "entries": entries.len(), "head": head_hash, "public_key": public_key, "bookmarks": bookmarks.len() }),
        format!(
            "The log of {repo} checks out: {} entries chained, head signed by {}, {} bookmarks match.\n",
            entries.len(),
            public_key.get(..16).unwrap_or(&public_key),
            bookmarks.len()
        ),
    ))
}
