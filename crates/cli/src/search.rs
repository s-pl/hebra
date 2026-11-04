//! `hb search`: find code by the words in it and around it.
//!
//! Files are cut into chunks (functions, types, methods, sections; see
//! [`hebra_core::structural::chunks`]) and ranked with BM25 in SQLite's
//! FTS5, which is already in the binary: nothing to download, no model.
//! Identifiers are split (`parseDate` is also `parse date`) and words are
//! stemmed. Change descriptions and the prompts behind them are indexed
//! too, so a question can match why code exists. Tests rank a little
//! lower than the code they test.
//!
//! The index lives in `.hebra/search.db`, outside history. Chunks are kept
//! by path and file content, so it updates only what changed, and every
//! workspace of the repository shares it. It can always be deleted.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use hebra_core::history::PathSpec;
use hebra_core::store::BLOB;
use hebra_core::structural;
use hebra_core::wc::is_binary;
use hebra_core::{Error, Tx};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::cli::SearchArgs;
use crate::commands::Output;
use crate::porcelain::Coded;

/// Files larger than this are not indexed.
const MAX_FILE: usize = 1024 * 1024;
/// Text kept per chunk, for ranking and previews.
const MAX_CHUNK: usize = 16 * 1024;
/// Commits of history read per search, newest first.
const MAX_CHANGES: usize = 20_000;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS files (path TEXT NOT NULL, blob TEXT NOT NULL, PRIMARY KEY (path, blob));
CREATE VIRTUAL TABLE IF NOT EXISTS chunks USING fts5(
    path, name, words, body,
    blob UNINDEXED, kind UNINDEXED, start UNINDEXED, finish UNINDEXED,
    tokenize = 'porter unicode61 remove_diacritics 2'
);
CREATE TABLE IF NOT EXISTS commits (id TEXT PRIMARY KEY);
CREATE VIRTUAL TABLE IF NOT EXISTS changes USING fts5(
    id UNINDEXED, description, why,
    tokenize = 'porter unicode61 remove_diacritics 2'
);
";

/// `camelCase` and `HTTPServer` as separate words; snake_case already is.
fn split_identifiers(text: &str) -> String {
    let mut out = String::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let chars: Vec<char> = word.chars().collect();
        if !chars.iter().any(|c| c.is_lowercase())
            || !chars.iter().skip(1).any(|c| c.is_uppercase())
        {
            continue;
        }
        for (i, c) in chars.iter().enumerate() {
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let prev = i.checked_sub(1).map(|p| chars[p]);
            let starts = c.is_uppercase()
                && prev.is_some_and(|p| {
                    p.is_lowercase() || p.is_numeric() || (p.is_uppercase() && next_lower)
                });
            if starts {
                out.push(' ');
            }
            out.extend(c.to_lowercase());
        }
        out.push(' ');
    }
    out
}

/// The query as FTS5 terms joined by OR: BM25 ranks chunks with more of
/// them, and rarer ones, first.
fn match_expr(query: &str) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let words = format!("{query} {}", split_identifiers(query));
    for word in words.split(|c: char| !c.is_alphanumeric()) {
        let word = word.to_lowercase();
        if word.is_empty() || !seen.insert(word.clone()) {
            continue;
        }
        terms.push(format!("\"{word}\""));
    }
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

fn open(index: &Path) -> Coded<Connection> {
    let db = Connection::open(index)?;
    db.busy_timeout(std::time::Duration::from_secs(10))?;
    db.execute_batch(SCHEMA)?;
    Ok(db)
}

/// What one search had to index first.
#[derive(Debug, Default)]
struct Indexed {
    files: usize,
    chunks: usize,
    changes: usize,
}

/// Indexes the files of `live` not indexed yet with that content.
fn index_files(
    tx: &mut Tx<'_>,
    db: &mut Connection,
    live: &[(String, hebra_core::ObjectId)],
) -> Coded<Indexed> {
    let mut done = Indexed::default();
    let known: HashSet<(String, String)> = {
        let mut statement = db.prepare("SELECT path, blob FROM files")?;
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    let todo: Vec<&(String, hebra_core::ObjectId)> = live
        .iter()
        .filter(|(path, blob)| !known.contains(&(path.clone(), blob.hex())))
        .collect();
    if todo.is_empty() {
        return Ok(done);
    }
    let before: HashSet<&str> = known.iter().map(|(p, _)| p.as_str()).collect();
    let stale: HashSet<&str> = todo
        .iter()
        .map(|(p, _)| p.as_str())
        .filter(|p| before.contains(p))
        .collect();
    let writer = db.transaction()?;
    {
        // Older versions of the same paths go: one copy per path is enough.
        let mut forget = writer.prepare("DELETE FROM chunks WHERE path = ?1")?;
        let mut unmark = writer.prepare("DELETE FROM files WHERE path = ?1")?;
        for path in &stale {
            forget.execute([path])?;
            unmark.execute([path])?;
        }
        let mut insert = writer.prepare(
            "INSERT INTO chunks (path, name, words, body, blob, kind, start, finish)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        let mut mark =
            writer.prepare("INSERT OR IGNORE INTO files (path, blob) VALUES (?1, ?2)")?;
        for (path, blob) in todo {
            mark.execute(params![path, blob.hex()])?;
            done.files += 1;
            let bytes = tx.store.read(BLOB, blob)?;
            if bytes.len() > MAX_FILE || is_binary(&bytes) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            for chunk in structural::chunks(path, &text) {
                let mut body = chunk.text;
                if body.len() > MAX_CHUNK {
                    let mut cut = MAX_CHUNK;
                    while !body.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    body.truncate(cut);
                }
                let name = chunk.name.unwrap_or_default();
                let words = format!("{} {}", split_identifiers(&name), split_identifiers(&body));
                let words = format!("{} {words}", split_identifiers(path));
                insert.execute(params![
                    path,
                    name,
                    words,
                    body,
                    blob.hex(),
                    chunk.kind,
                    i64::try_from(chunk.start).unwrap_or(i64::MAX),
                    i64::try_from(chunk.end).unwrap_or(i64::MAX),
                ])?;
                done.chunks += 1;
            }
        }
    }
    writer.commit()?;
    Ok(done)
}

/// Indexes the descriptions and prompts of these commits.
fn index_changes(
    tx: &mut Tx<'_>,
    db: &mut Connection,
    ids: &[hebra_core::ObjectId],
) -> Coded<usize> {
    let known: HashSet<String> = {
        let mut statement = db.prepare("SELECT id FROM commits")?;
        statement
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?
    };
    let writer = db.transaction()?;
    let mut added = 0;
    {
        let mut mark = writer.prepare("INSERT OR IGNORE INTO commits (id) VALUES (?1)")?;
        let mut insert =
            writer.prepare("INSERT INTO changes (id, description, why) VALUES (?1, ?2, ?3)")?;
        for id in ids {
            if *id == tx.root_commit || known.contains(&id.hex()) {
                continue;
            }
            let commit = tx.commit(id)?;
            mark.execute([id.hex()])?;
            let p = &commit.provenance;
            let why = [p.task.as_deref(), p.prompt.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n");
            if commit.description.trim().is_empty() && why.is_empty() {
                continue;
            }
            insert.execute(params![id.hex(), commit.description, why])?;
            added += 1;
        }
    }
    writer.commit()?;
    Ok(added)
}

/// The first line of a chunk that says something: not a comment, not a
/// decorator.
fn preview(body: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| {
            !l.is_empty()
                && !l.starts_with("//")
                && !l.starts_with('#')
                && !l.starts_with("/*")
                && !l.starts_with('*')
                && !l.starts_with("--")
        })
        .or_else(|| body.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or_default();
    let mut line = line.to_owned();
    if line.len() > 120 {
        let mut cut = 117;
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        line.truncate(cut);
        line.push_str("...");
    }
    line
}

pub fn run(tx: &mut Tx<'_>, index: &Path, args: &SearchArgs) -> Coded<Output> {
    let query = args.query.join(" ");
    let expr = match_expr(&query).ok_or_else(|| {
        Error::InvalidArgument("say what to look for: `hb search parse date`".to_owned())
    })?;
    let started = Instant::now();
    let tip = tx.resolve(args.revision.as_deref().unwrap_or("@"))?;
    let tree = tx.commit_tree(&tip)?;
    let spec = PathSpec::new(&args.paths)?;
    let live: Vec<(String, hebra_core::ObjectId)> = tree
        .files
        .iter()
        .filter(|(path, entry)| entry.is_file() && spec.matches(path))
        .map(|(path, entry)| (path.clone(), entry.blob))
        .collect();

    let mut db = open(index)?;
    let mut indexed = index_files(tx, &mut db, &live)?;
    let ancestry: Vec<hebra_core::ObjectId> = if args.code_only {
        Vec::new()
    } else {
        tx.walk_from(&[tip], MAX_CHANGES)?
    };
    indexed.changes = index_changes(tx, &mut db, &ancestry)?;

    db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS live (path TEXT, blob TEXT, PRIMARY KEY (path, blob)); DELETE FROM live;")?;
    {
        let mut insert = db.prepare("INSERT OR IGNORE INTO live (path, blob) VALUES (?1, ?2)")?;
        for (path, blob) in &live {
            insert.execute(params![path, blob.hex()])?;
        }
    }
    let limit = i64::try_from(args.limit.max(1)).unwrap_or(10);
    let mut results = Vec::new();
    {
        let mut statement = db.prepare(
            "SELECT chunks.path, chunks.name, chunks.kind, chunks.start, chunks.finish, chunks.body,
                    bm25(chunks, 3.0, 8.0, 1.0, 1.0) * CASE
                        WHEN chunks.name LIKE 'tests::%' OR chunks.path LIKE '%tests/%'
                          OR chunks.path LIKE '%test.%' OR chunks.path LIKE '%spec.%' THEN 0.6
                        ELSE 1.0 END AS score
             FROM chunks JOIN live ON live.path = chunks.path AND live.blob = chunks.blob
             WHERE chunks MATCH ?1
             ORDER BY score LIMIT ?2",
        )?;
        let rows = statement.query_map(params![expr, limit], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, f64>(6)?,
            ))
        })?;
        for row in rows {
            results.push(row?);
        }
    }

    let mut changes = Vec::new();
    if !args.code_only {
        let mut statement = db.prepare(
            "SELECT id, description, why, bm25(changes, 2.0, 1.0) AS score
             FROM changes WHERE changes MATCH ?1 ORDER BY score LIMIT ?2",
        )?;
        let rows = statement.query_map(params![expr, 200], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, f64>(3)?,
            ))
        })?;
        let ancestry: HashSet<String> = ancestry.iter().map(hebra_core::ObjectId::hex).collect();
        for row in rows {
            let row = row?;
            if ancestry.contains(&row.0) && changes.len() < 5 {
                changes.push(row);
            }
        }
    }

    let ms = started.elapsed().as_millis();
    let mut text = String::new();
    for (path, name, _, start, end, body, _) in &results {
        let label = if name.is_empty() {
            String::new()
        } else {
            format!("  {name}")
        };
        let _ = writeln!(text, "{path}:{start}-{end}{label}");
        let _ = writeln!(text, "    {}", preview(body));
    }
    if !changes.is_empty() {
        let _ = writeln!(text, "\nChanges");
        for (id, description, ..) in &changes {
            let short = &id[..12.min(id.len())];
            let _ = writeln!(
                text,
                "  {short}  {}",
                description.lines().next().unwrap_or_default()
            );
        }
    }
    if results.is_empty() && changes.is_empty() {
        let _ = writeln!(text, "Nothing matches {query:?}.");
    }
    let json_results: Vec<Value> = results
        .iter()
        .map(|(path, name, kind, start, end, body, score)| {
            json!({
                "path": path,
                "name": (!name.is_empty()).then_some(name),
                "kind": kind,
                "start": start,
                "end": end,
                "preview": preview(body),
                "score": (-score * 100.0).round() / 100.0,
            })
        })
        .collect();
    let json_changes: Vec<Value> = changes
        .iter()
        .map(|(id, description, why, score)| {
            json!({
                "commit": id,
                "description": description,
                "why": (!why.is_empty()).then_some(why),
                "score": (-score * 100.0).round() / 100.0,
            })
        })
        .collect();
    let found = results.len() + changes.len();
    Ok(Output::with_exit(
        json!({
            "query": query,
            "results": json_results,
            "changes": json_changes,
            "indexed": { "files": indexed.files, "chunks": indexed.chunks, "changes": indexed.changes },
            "ms": ms,
        }),
        text,
        u8::from(found == 0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_split_into_words() {
        assert_eq!(
            split_identifiers("parseDate HTTPServer x snake_case").trim(),
            "parse date http server"
        );
        assert_eq!(
            match_expr("parseDate now").as_deref(),
            Some("\"parsedate\" OR \"now\" OR \"parse\" OR \"date\"")
        );
        assert_eq!(match_expr("  ,, "), None);
    }
}
