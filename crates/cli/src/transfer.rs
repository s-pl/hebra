//! Moving changes without a server: patches by mail, archives and bundles.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use hebra_core::archive::{self, Format};
use hebra_core::diff::{self, DiffOptions};
use hebra_core::model::{Actor, Commit, FileEntry, Provenance, Tree};
use hebra_core::store::BLOB;
use hebra_core::{Error, ObjectId, Tx, Workspace, transport};
use serde_json::{Value, json};

use crate::commands::Output;
use crate::porcelain::{Coded, expand_range, parse_person, sign_one};
use crate::sign::Signer;

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidArgument(message.into())
}

fn slug(subject: &str) -> String {
    let clean: String = subject
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let mut out = String::new();
    for part in clean.split('-').filter(|p| !p.is_empty()) {
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(part);
        if out.len() > 50 {
            break;
        }
    }
    out
}

fn rfc2822(timestamp: &str) -> String {
    timestamp
        .parse::<jiff::Timestamp>()
        .ok()
        .and_then(|t| t.strftime("%a, %d %b %Y %H:%M:%S +0000").to_string().into())
        .unwrap_or_else(|| timestamp.to_owned())
}

/// One change as a mail, in the format of `git format-patch`.
fn patch_mail(tx: &mut Tx<'_>, id: &ObjectId, n: usize, total: usize) -> Coded<(String, String)> {
    let commit = tx.commit(id)?;
    let (base, _) = tx.parents_tree(&commit.parents)?;
    let tree = tx.tree(&commit.tree)?;
    let files = diff::diff_with(
        tx.store,
        &base,
        &tree,
        DiffOptions {
            with_patch: true,
            context: 3,
            renames: false,
        },
    )?;
    let subject = commit
        .description
        .lines()
        .next()
        .unwrap_or("(no description)");
    let body = commit
        .description
        .split_once("\n\n")
        .map_or("", |(_, rest)| rest);
    let email = commit.author.email.as_deref().unwrap_or("hebra@localhost");
    let mut mail = String::new();
    let _ = writeln!(mail, "From {id} Mon Sep 17 00:00:00 2001");
    let _ = writeln!(mail, "From: {} <{email}>", commit.author.name);
    let date = commit.authored_at.as_deref().unwrap_or(&commit.timestamp);
    let _ = writeln!(mail, "Date: {}", rfc2822(date));
    if total > 1 {
        let _ = writeln!(mail, "Subject: [PATCH {n}/{total}] {subject}");
    } else {
        let _ = writeln!(mail, "Subject: [PATCH] {subject}");
    }
    mail.push('\n');
    if !body.is_empty() {
        let _ = writeln!(mail, "{body}\n");
    }
    mail.push_str("---\n");
    for f in &files {
        let _ = writeln!(mail, " {} | {}", f.path, f.additions + f.deletions);
    }
    mail.push('\n');
    for f in &files {
        mail.push_str(&diff::git_header(f));
        if let Some(patch) = &f.patch {
            mail.push_str(patch);
        }
    }
    mail.push_str("-- \nhb\n\n");
    let name = format!("{n:04}-{}.patch", slug(subject));
    Ok((name, mail))
}

pub fn format_patch(tx: &mut Tx<'_>, revision: &str, dir: &Path, stdout: bool) -> Coded<Output> {
    let ids = if revision.contains("..") {
        expand_range(tx, revision)?
    } else {
        let from = tx.resolve(revision)?;
        let to = tx.resolve("@-")?;
        tx.range(Some(from), to)?
    };
    let total = ids.len();
    let mut text = String::new();
    let mut names = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let (name, mail) = patch_mail(tx, id, i + 1, total)?;
        if stdout {
            text.push_str(&mail);
        } else {
            let path = dir.join(&name);
            std::fs::write(&path, mail).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            let _ = writeln!(text, "{}", path.display());
            names.push(path.display().to_string());
        }
    }
    Ok(Output::new(json!({ "patches": names }), text))
}

/// One file's part of a patch.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FilePatch {
    old: Option<String>,
    new: Option<String>,
    /// From the `---` line on.
    text: String,
}

fn header_path(line: &str) -> Option<String> {
    let path = line.split('\t').next().unwrap_or(line).trim();
    if path == "/dev/null" {
        return None;
    }
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    Some(path.to_owned())
}

fn hunk_lengths(line: &str) -> Option<(usize, usize)> {
    let inner = line.strip_prefix("@@ -")?;
    let (old, rest) = inner.split_once(" +")?;
    let new = rest.split(" @@").next()?;
    let count = |range: &str| {
        range
            .split_once(',')
            .map_or(Some(1), |(_, n)| n.parse().ok())
    };
    Some((count(old)?, count(new)?))
}

/// Splits a unified diff by file, counting hunk lines: a removed line can
/// look exactly like a header.
fn split_patch(text: &str) -> Vec<FilePatch> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let is_header =
            lines[i].starts_with("--- ") && lines.get(i + 1).is_some_and(|l| l.starts_with("+++ "));
        if !is_header {
            i += 1;
            continue;
        }
        let old = header_path(&lines[i][4..]);
        let new = header_path(&lines[i + 1][4..]);
        let start = i;
        i += 2;
        while let Some((mut a, mut b)) = lines.get(i).and_then(|l| hunk_lengths(l)) {
            i += 1;
            while (a > 0 || b > 0) && i < lines.len() {
                match lines[i].as_bytes().first() {
                    Some(b'+') => b -= 1,
                    Some(b'-') => a -= 1,
                    Some(b'\\') => {}
                    _ => {
                        a = a.saturating_sub(1);
                        b = b.saturating_sub(1);
                    }
                }
                i += 1;
            }
            if lines.get(i).is_some_and(|l| l.starts_with('\\')) {
                i += 1;
            }
        }
        let mut text = lines[start..i].join("\n");
        text.push('\n');
        out.push(FilePatch { old, new, text });
    }
    out
}

fn apply_text(tx: &Tx<'_>, tree: &mut Tree, patch: &FilePatch, reverse: bool) -> Coded<()> {
    for path in [&patch.old, &patch.new].into_iter().flatten() {
        hebra_core::wc::safe_join(Path::new("workspace"), path)?;
    }
    let parsed =
        diffy::Patch::from_str(&patch.text).map_err(|e| invalid(format!("invalid patch: {e}")))?;
    let parsed = if reverse { parsed.reverse() } else { parsed };
    let (old, new) = if reverse {
        (&patch.new, &patch.old)
    } else {
        (&patch.old, &patch.new)
    };
    let current = match old {
        Some(path) => {
            let entry = tree
                .files
                .get(path)
                .ok_or_else(|| invalid(format!("{path} does not exist")))?;
            String::from_utf8_lossy(&tx.store.read(BLOB, &entry.blob)?).into_owned()
        }
        None => String::new(),
    };
    let result = diffy::apply(&current, &parsed).map_err(|e| {
        invalid(format!(
            "patch does not apply to {}: {e}",
            old.as_deref().or(new.as_deref()).unwrap_or("?")
        ))
    })?;
    let executable = old
        .as_ref()
        .and_then(|p| tree.files.get(p))
        .is_some_and(|e| e.executable);
    if let Some(path) = old {
        tree.files.remove(path);
    }
    if let Some(path) = new {
        let blob = tx.store.write(BLOB, result.as_bytes())?;
        tree.files
            .insert(path.clone(), FileEntry::file(blob, executable));
    }
    Ok(())
}

pub fn apply(tx: &mut Tx<'_>, texts: &[String], check: bool, reverse: bool) -> Coded<Output> {
    let wc = tx.wc_id()?;
    let mut tree = tx.commit_tree(&wc)?;
    let mut touched = Vec::new();
    for text in texts {
        let patches = split_patch(text);
        if patches.is_empty() {
            return Err(invalid("no patch found"));
        }
        for patch in patches {
            apply_text(tx, &mut tree, &patch, reverse)?;
            touched.push(patch.new.clone().or(patch.old.clone()).unwrap_or_default());
        }
    }
    if check {
        return Ok(Output::new(
            json!({ "applies": true, "files": touched }),
            String::new(),
        ));
    }
    tx.set_wc_tree(&tree, Vec::new())?;
    let text: String = touched
        .iter()
        .map(|p| format!("Applied to {p}\n"))
        .collect();
    Ok(Output::new(json!({ "files": touched }), text))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub author: Option<Actor>,
    pub date: Option<String>,
    pub subject: String,
    pub body: String,
    pub patch: String,
}

fn parse_date_header(text: &str) -> Option<String> {
    jiff::fmt::rfc2822::parse(text.trim())
        .ok()
        .map(|z| z.timestamp().to_string())
}

/// Reads `git format-patch` mails: one or many in a file.
pub fn parse_mails(text: &str) -> Vec<Mail> {
    let mut chunks: Vec<Vec<&str>> = Vec::new();
    for line in text.lines() {
        if line.starts_with("From ") && line.ends_with("2001") {
            chunks.push(Vec::new());
        } else if let Some(chunk) = chunks.last_mut() {
            chunk.push(line);
        }
    }
    if chunks.is_empty() {
        chunks.push(text.lines().collect());
    }
    chunks
        .into_iter()
        .map(|lines| {
            let mut mail = Mail {
                author: None,
                date: None,
                subject: String::new(),
                body: String::new(),
                patch: String::new(),
            };
            let mut i = 0;
            while i < lines.len() && !lines[i].is_empty() {
                let line = lines[i];
                if let Some(rest) = line.strip_prefix("From: ") {
                    mail.author = Some(parse_person(rest));
                } else if let Some(rest) = line.strip_prefix("Date: ") {
                    mail.date = parse_date_header(rest);
                } else if let Some(rest) = line.strip_prefix("Subject: ") {
                    let clean = rest
                        .trim_start_matches("[PATCH")
                        .split_once(']')
                        .map_or(rest, |(_, s)| s);
                    mail.subject = clean.trim().to_owned();
                }
                i += 1;
            }
            let separator = lines
                .iter()
                .position(|l| *l == "---")
                .unwrap_or(lines.len());
            mail.body = lines[i.min(separator)..separator]
                .join("\n")
                .trim()
                .to_owned();
            if separator < lines.len() {
                mail.patch = lines[separator..].join("\n");
                mail.patch.push('\n');
            }
            mail
        })
        .collect()
}

pub fn am(tx: &mut Tx<'_>, mails: &[Mail], signer: Option<&Signer>) -> Coded<Output> {
    let mut text = String::new();
    let mut done = Vec::new();
    for mail in mails {
        let wc = tx.wc_id()?;
        let mut tree = tx.commit_tree(&wc)?;
        for patch in split_patch(&mail.patch) {
            apply_text(tx, &mut tree, &patch, false)?;
        }
        let description = if mail.body.is_empty() {
            mail.subject.clone()
        } else {
            format!("{}\n\n{}", mail.subject, mail.body)
        };
        let tree_id = tx.store.write_tree(&tree)?;
        let (author, date) = (mail.author.clone(), mail.date.clone());
        let committed = tx.rewrite(wc, |c: &mut Commit| {
            c.tree = tree_id;
            c.description = description;
            if let Some(author) = author {
                c.author = author;
            }
            c.authored_at = date;
        })?;
        let next = tx.new_commit(vec![committed], tree_id, String::new(), Vec::new())?;
        tx.switch_wc(next)?;
        if let Some(name) = tx.current_bookmark() {
            tx.view.bookmarks.insert(name, committed);
        }
        let committed = sign_one(tx, committed, signer)?;
        let _ = writeln!(text, "Applied: {}", mail.subject);
        done.push(committed.hex());
    }
    Ok(Output::new(json!({ "applied": done }), text))
}

pub fn archive_cmd(
    tx: &mut Tx<'_>,
    revision: &str,
    format: Option<&str>,
    output: Option<&Path>,
    prefix: Option<&str>,
) -> Coded<Output> {
    let id = tx.resolve(revision)?;
    let commit = tx.commit(&id)?;
    let tree = tx.tree(&commit.tree)?;
    let format = match (format, output) {
        (Some(name), _) => Format::parse(name)?,
        (None, Some(path)) => Format::for_file(&path.to_string_lossy()),
        (None, None) => Format::Tar,
    };
    let mtime = commit
        .timestamp
        .parse::<jiff::Timestamp>()
        .map_or(0, |t| u64::try_from(t.as_second()).unwrap_or(0));
    let mut bytes = Vec::new();
    archive::write(
        tx.store,
        &tree,
        format,
        prefix.unwrap_or(""),
        mtime,
        &mut bytes,
    )?;
    match output {
        Some(path) => {
            std::fs::write(path, &bytes).map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;
            Ok(Output::new(
                json!({ "file": path.display().to_string(), "bytes": bytes.len() }),
                format!("Wrote {} ({} bytes)\n", path.display(), bytes.len()),
            ))
        }
        None => Ok(Output::binary(json!({ "bytes": bytes.len() }), bytes)),
    }
}

// ---------------------------------------------------------------- bundle

const BUNDLE_MAGIC: &str = "HEBRA-BUNDLE-1\n";

pub fn bundle_create(ws: &Workspace, file: &Path, names: &[String]) -> Result<Output> {
    let view = ws.view()?;
    let bookmarks: BTreeMap<String, ObjectId> = view
        .bookmarks
        .iter()
        .filter(|(n, _)| names.is_empty() || names.contains(n))
        .map(|(n, id)| (n.clone(), *id))
        .collect();
    let wants: Vec<ObjectId> = bookmarks.values().copied().collect();
    let ids = transport::closure(&ws.store, &wants, &HashSet::new())?;
    let objects = transport::collect(&ws.store, &ids)?;
    let header = json!({ "bookmarks": bookmarks, "tags": view.tags });
    let mut out = BUNDLE_MAGIC.as_bytes().to_vec();
    out.extend_from_slice(header.to_string().as_bytes());
    out.push(b'\n');
    out.extend_from_slice(&transport::encode(&objects));
    std::fs::write(file, out).with_context(|| format!("writing {}", file.display()))?;
    Ok(Output::new(
        json!({ "file": file.display().to_string(), "objects": objects.len(), "bookmarks": bookmarks }),
        format!(
            "Bundled {} bookmark(s), {} object(s) into {}\n",
            bookmarks.len(),
            objects.len(),
            file.display()
        ),
    ))
}

fn bundle_read(file: &Path) -> Result<(Value, Vec<u8>)> {
    let raw = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let rest = raw
        .strip_prefix(BUNDLE_MAGIC.as_bytes())
        .ok_or_else(|| anyhow!("{} is not a bundle", file.display()))?;
    let end = rest
        .iter()
        .position(|b| *b == b'\n')
        .ok_or_else(|| anyhow!("truncated bundle"))?;
    Ok((
        serde_json::from_slice(&rest[..end])?,
        rest[end + 1..].to_vec(),
    ))
}

pub fn bundle_heads(file: &Path) -> Result<Output> {
    let (header, pack) = bundle_read(file)?;
    transport::decode(&pack)?;
    let mut text = String::new();
    for (name, id) in header["bookmarks"].as_object().into_iter().flatten() {
        let _ = writeln!(text, "{} refs/heads/{name}", id.as_str().unwrap_or(""));
    }
    Ok(Output::new(header, text))
}

pub fn bundle_unbundle(
    ws: &mut Workspace,
    actor: &Actor,
    prov: &Provenance,
    file: &Path,
) -> Result<Output> {
    let (header, pack) = bundle_read(file)?;
    let objects = transport::decode(&pack)?;
    let store = ws.store.clone();
    let new = store.batch(|| transport::store_all(&store, &objects))?;
    let mut bookmarks = BTreeMap::new();
    for (name, id) in header["bookmarks"].as_object().into_iter().flatten() {
        bookmarks.insert(
            name.clone(),
            ObjectId::parse(id.as_str().unwrap_or_default())?,
        );
    }
    let tags = serde_json::from_value(header["tags"].clone()).unwrap_or_default();
    let names: Vec<String> = bookmarks.keys().cloned().collect();
    ws.run(actor, prov, "bundle", |tx| {
        tx.view.remotes.insert("bundle".to_owned(), bookmarks);
        let tags: BTreeMap<String, hebra_core::model::Tag> = tags;
        for (name, tag) in tags {
            tx.view.tags.entry(name).or_insert(tag);
        }
        Ok(())
    })?;
    Ok(Output::new(
        json!({ "objects": new, "bookmarks": names }),
        format!(
            "Loaded {new} new object(s). Bookmarks are at <name>@bundle: {}\n",
            names.join(", ")
        ),
    ))
}

/// Reads the files named on the command line (`-` is standard input).
pub fn read_inputs(cwd: &Path, files: &[PathBuf]) -> Result<Vec<String>> {
    files
        .iter()
        .map(|file| {
            if file.as_os_str() == "-" {
                let mut text = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
                return Ok(text);
            }
            let path = cwd.join(file);
            std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
        })
        .collect()
}

pub fn require_mails(mails: &[Mail]) -> Result<()> {
    if mails.iter().all(|m| m.patch.is_empty()) {
        bail!("no patch found");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_split_by_file_even_with_lines_that_look_like_headers() {
        let text = "--- a/one.txt\n+++ b/one.txt\n@@ -1,2 +1,1 @@\n--- not a header\n kept\n--- /dev/null\n+++ b/two.txt\n@@ -0,0 +1 @@\n+new\n";
        let files = split_patch(text);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].old.as_deref(), Some("one.txt"));
        assert_eq!(files[1].old, None);
        assert_eq!(files[1].new.as_deref(), Some("two.txt"));
    }

    #[test]
    fn mails_give_back_author_subject_and_body() {
        let text = "From abc Mon Sep 17 00:00:00 2001\nFrom: Ana <ana@x.dev>\nDate: Fri, 02 Jan 2026 03:04:05 +0000\nSubject: [PATCH 1/2] add thing\n\nlonger text\n\n---\n a | 1\n\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n+y\n-- \nhb\n";
        let mails = parse_mails(text);
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].subject, "add thing");
        assert_eq!(mails[0].body, "longer text");
        assert_eq!(mails[0].author.as_ref().unwrap().name, "Ana");
        assert!(mails[0].date.as_ref().unwrap().starts_with("2026-01-02"));
        assert_eq!(split_patch(&mails[0].patch).len(), 1);
    }
}
