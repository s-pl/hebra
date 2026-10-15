//! Import Git history from a `git fast-export --show-original-ids` stream.
//!
//! Change ids are derived from the Git commit ids, so importing the same
//! history twice gives the same Hebra commits. Commits that were exported by
//! Hebra (`Hebra-*` trailers) get their original change id and provenance
//! back, so a round trip through Git loses nothing.

use std::collections::HashMap;
use std::io::BufRead;

use crate::error::{Error, Result};
use crate::ids::{ChangeId, ObjectId};
use crate::model::{Actor, ActorKind, Commit, FileEntry, Provenance, Tree};
use crate::repo::Tx;
use crate::store::{BLOB, TREE};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ImportStats {
    pub commits: usize,
    pub blobs: usize,
    pub skipped_entries: usize,
}

fn bad(why: impl Into<String>) -> Error {
    Error::InvalidArgument(format!("unexpected git fast-export input: {}", why.into()))
}

struct Parser<R> {
    input: R,
    peeked: Option<Vec<u8>>,
}

impl<R: BufRead> Parser<R> {
    fn line(&mut self) -> Result<Option<Vec<u8>>> {
        if let Some(line) = self.peeked.take() {
            return Ok(Some(line));
        }
        let mut buf = Vec::new();
        let n = self
            .input
            .read_until(b'\n', &mut buf)
            .map_err(|source| Error::Io {
                path: "git fast-export".into(),
                source,
            })?;
        if n == 0 {
            return Ok(None);
        }
        if buf.last() == Some(&b'\n') {
            buf.pop();
        }
        Ok(Some(buf))
    }

    fn peek(&mut self) -> Result<Option<&[u8]>> {
        if self.peeked.is_none() {
            self.peeked = self.line()?;
        }
        Ok(self.peeked.as_deref())
    }

    /// Reads a `data <n>` block (fast-export always uses exact counts).
    fn data(&mut self, header: &[u8]) -> Result<Vec<u8>> {
        let count = std::str::from_utf8(
            header
                .strip_prefix(b"data ")
                .ok_or_else(|| bad("expected data"))?,
        )
        .ok()
        .and_then(|n| n.trim().parse::<usize>().ok())
        .ok_or_else(|| bad("data length"))?;
        let mut buf = vec![0u8; count];
        std::io::Read::read_exact(&mut self.input, &mut buf).map_err(|source| Error::Io {
            path: "git fast-export".into(),
            source,
        })?;
        Ok(buf)
    }
}

/// Undoes Git's C-style path quoting.
fn unquote(path: &[u8]) -> Result<String> {
    let Some(inner) = path.strip_prefix(b"\"").and_then(|p| p.strip_suffix(b"\"")) else {
        return String::from_utf8(path.to_vec()).map_err(|_| bad("path is not UTF-8"));
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < inner.len() {
        if inner[i] != b'\\' || i + 1 >= inner.len() {
            out.push(inner[i]);
            i += 1;
            continue;
        }
        let c = inner[i + 1];
        i += 2;
        match c {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'"' => out.push(b'"'),
            b'\\' => out.push(b'\\'),
            b'0'..=b'7' => {
                let digits = &inner[i - 1..(i + 2).min(inner.len())];
                let value = std::str::from_utf8(digits)
                    .ok()
                    .and_then(|d| u8::from_str_radix(d, 8).ok())
                    .ok_or_else(|| bad("octal escape"))?;
                out.push(value);
                i += 2;
            }
            other => out.push(other),
        }
    }
    String::from_utf8(out).map_err(|_| bad("path is not UTF-8"))
}

/// Splits `Name <email> 1700000000 +0100` into the name and the time.
fn parse_identity(line: &[u8]) -> (String, i64) {
    let text = String::from_utf8_lossy(line);
    let name = text.split('<').next().unwrap_or("").trim().to_owned();
    let seconds = text
        .rsplit('>')
        .next()
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    (name, seconds)
}

struct Message {
    description: String,
    change_id: Option<ChangeId>,
    agent: Option<String>,
    on_behalf_of: Option<String>,
    provenance: Provenance,
}

/// Separates the description from Hebra trailers written by `hb git export`.
fn parse_message(raw: &[u8]) -> Message {
    let text = String::from_utf8_lossy(raw).into_owned();
    let mut lines: Vec<&str> = text.trim_end().lines().collect();
    let mut msg = Message {
        description: String::new(),
        change_id: None,
        agent: None,
        on_behalf_of: None,
        provenance: Provenance::default(),
    };
    while let Some(last) = lines.last() {
        let Some((key, value)) = last.split_once(": ") else {
            break;
        };
        if !key.starts_with("Hebra-") {
            break;
        }
        let value = value.trim().to_owned();
        match key {
            "Hebra-Change" => msg.change_id = ChangeId::parse(&value).ok(),
            "Hebra-Agent" => msg.agent = Some(value),
            "Hebra-On-Behalf-Of" => msg.on_behalf_of = Some(value),
            "Hebra-Task" => msg.provenance.task = Some(value),
            "Hebra-Session" => msg.provenance.session = Some(value),
            _ => {}
        }
        lines.pop();
    }
    let description = lines.join("\n").trim_end().to_owned();
    msg.description = if description == "(no description)" {
        String::new()
    } else {
        description
    };
    msg
}

/// Imports a fast-export stream. Returns the stats and the Hebra commit of
/// the last commit in the stream (the branch tip).
pub fn import(tx: &mut Tx<'_>, input: impl BufRead) -> Result<(ImportStats, Option<ObjectId>)> {
    let mut p = Parser {
        input,
        peeked: None,
    };
    let mut blobs: HashMap<String, ObjectId> = HashMap::new();
    let mut commits: HashMap<String, ObjectId> = HashMap::new();
    let mut trees: HashMap<ObjectId, Tree> = HashMap::new();
    let mut stats = ImportStats {
        commits: 0,
        blobs: 0,
        skipped_entries: 0,
    };
    let mut tip = None;

    while let Some(line) = p.line()? {
        if line.is_empty() || line.starts_with(b"feature ") || line.starts_with(b"progress ") {
            continue;
        }
        if line == b"done" {
            break;
        }
        if line == b"blob" {
            let mut mark = None;
            loop {
                let l = p.line()?.ok_or_else(|| bad("eof in blob"))?;
                if let Some(m) = l.strip_prefix(b"mark ") {
                    mark = Some(String::from_utf8_lossy(m).into_owned());
                } else if l.starts_with(b"original-oid ") {
                } else if l.starts_with(b"data ") {
                    let bytes = p.data(&l)?;
                    let id = tx.store.write(BLOB, &bytes)?;
                    if let Some(m) = mark.take() {
                        blobs.insert(m, id);
                    }
                    stats.blobs += 1;
                    break;
                } else {
                    return Err(bad(format!("in blob: {}", String::from_utf8_lossy(&l))));
                }
            }
            continue;
        }
        if line.starts_with(b"reset ") {
            if p.peek()?.is_some_and(|l| l.starts_with(b"from ")) {
                p.line()?;
            }
            continue;
        }
        if line.starts_with(b"tag ") {
            // Tags are not imported; skip their header and message.
            while let Some(l) = p.line()? {
                if l.starts_with(b"data ") {
                    p.data(&l)?;
                    break;
                }
            }
            continue;
        }
        let Some(_branch) = line.strip_prefix(b"commit ") else {
            return Err(bad(String::from_utf8_lossy(&line).into_owned()));
        };

        let mut mark = None;
        let mut original = None;
        let mut author = (String::from("unknown"), 0i64);
        let mut committer_time = 0i64;
        let message;
        loop {
            let l = p.line()?.ok_or_else(|| bad("eof in commit header"))?;
            if let Some(m) = l.strip_prefix(b"mark ") {
                mark = Some(String::from_utf8_lossy(m).into_owned());
            } else if let Some(o) = l.strip_prefix(b"original-oid ") {
                original = Some(o.to_vec());
            } else if let Some(a) = l.strip_prefix(b"author ") {
                author = parse_identity(a);
            } else if let Some(c) = l.strip_prefix(b"committer ") {
                committer_time = parse_identity(c).1;
            } else if l.starts_with(b"encoding ") {
            } else if l.starts_with(b"gpgsig ") {
                let data = p.line()?.ok_or_else(|| bad("eof in gpgsig"))?;
                p.data(&data)?;
            } else if l.starts_with(b"data ") {
                message = p.data(&l)?;
                break;
            } else {
                return Err(bad(format!("in commit: {}", String::from_utf8_lossy(&l))));
            }
        }

        let mut parents = Vec::new();
        let mut tree = Tree::default();
        let mut first = true;
        loop {
            let Some(l) = p.peek()? else { break };
            if l.is_empty() {
                p.line()?;
                break;
            }
            let l = l.to_vec();
            if let Some(from) = l
                .strip_prefix(b"from ")
                .or_else(|| l.strip_prefix(b"merge "))
            {
                let key = String::from_utf8_lossy(from).into_owned();
                let parent = *commits
                    .get(&key)
                    .ok_or_else(|| bad(format!("unknown parent {key}")))?;
                if first {
                    tree = trees.get(&parent).cloned().unwrap_or_default();
                    first = false;
                }
                parents.push(parent);
            } else if l == b"deleteall" {
                tree = Tree::default();
            } else if let Some(rest) = l.strip_prefix(b"M ") {
                let mut parts = rest.splitn(3, |&b| b == b' ');
                let mode = parts.next().unwrap_or_default();
                let dataref =
                    String::from_utf8_lossy(parts.next().unwrap_or_default()).into_owned();
                let path = unquote(parts.next().unwrap_or_default())?;
                match mode {
                    b"100644" | b"644" | b"100755" | b"755" => {
                        let blob = *blobs
                            .get(&dataref)
                            .ok_or_else(|| bad(format!("unknown blob {dataref}")))?;
                        let executable = mode.ends_with(b"755");
                        tree.files.insert(path, FileEntry { blob, executable });
                    }
                    // Symlinks and submodules are not tracked yet.
                    _ => stats.skipped_entries += 1,
                }
            } else if let Some(path) = l.strip_prefix(b"D ") {
                let path = unquote(path)?;
                tree.files.remove(&path);
                let prefix = format!("{path}/");
                tree.files.retain(|p, _| !p.starts_with(&prefix));
            } else if l.starts_with(b"commit ")
                || l.starts_with(b"reset ")
                || l == b"blob"
                || l == b"done"
            {
                break;
            } else {
                return Err(bad(format!(
                    "unsupported file command: {}",
                    String::from_utf8_lossy(&l)
                )));
            }
            p.line()?;
        }
        if parents.is_empty() {
            parents.push(tx.root_commit);
        }

        let msg = parse_message(&message);
        let change_id = msg.change_id.unwrap_or_else(|| {
            ChangeId::derived_from(original.as_deref().unwrap_or(message.as_slice()))
        });
        let (author_name, author_time) = author;
        let author = match msg.agent {
            Some(agent) => Actor {
                name: agent,
                kind: ActorKind::Agent,
                on_behalf_of: msg.on_behalf_of,
            },
            None => Actor::human(author_name),
        };
        let seconds = if committer_time != 0 {
            committer_time
        } else {
            author_time
        };
        let timestamp = jiff::Timestamp::from_second(seconds)
            .map_or_else(|_| "1970-01-01T00:00:00Z".to_owned(), |t| t.to_string());
        let tree_id = tx.store.write_json(TREE, &tree)?;
        let id = tx.write_commit(Commit {
            change_id,
            parents,
            predecessors: Vec::new(),
            tree: tree_id,
            description: msg.description,
            author,
            timestamp,
            provenance: msg.provenance,
            conflicts: Vec::new(),
        })?;
        trees.insert(id, tree);
        if let Some(m) = mark {
            commits.insert(m, id);
        }
        if let Some(o) = &original {
            commits.insert(String::from_utf8_lossy(o).into_owned(), id);
        }
        stats.commits += 1;
        tip = Some(id);
    }
    Ok((stats, tip))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquotes_git_paths() {
        assert_eq!(unquote(b"plain/path.rs").unwrap(), "plain/path.rs");
        assert_eq!(
            unquote(b"\"with \\\"quotes\\\"\"").unwrap(),
            "with \"quotes\""
        );
        assert_eq!(unquote(b"\"caf\\303\\251.txt\"").unwrap(), "café.txt");
    }

    #[test]
    fn reads_hebra_trailers_back() {
        let msg = parse_message(
            b"add OAuth\n\nHebra-Change: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\nHebra-Agent: claude-code\nHebra-On-Behalf-Of: ana\nHebra-Task: oauth\n",
        );
        assert_eq!(msg.description, "add OAuth");
        assert_eq!(msg.agent.as_deref(), Some("claude-code"));
        assert_eq!(msg.on_behalf_of.as_deref(), Some("ana"));
        assert_eq!(msg.provenance.task.as_deref(), Some("oauth"));
        assert_eq!(msg.change_id, Some(ChangeId::root()));
    }

    #[test]
    fn parses_identities() {
        assert_eq!(
            parse_identity(b"Ana Lopez <ana@x.dev> 1700000000 +0100"),
            ("Ana Lopez".to_owned(), 1_700_000_000)
        );
    }
}
