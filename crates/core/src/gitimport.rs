//! Import Git history from a `git fast-export --show-original-ids` stream.
//!
//! Change ids are derived from the Git commit ids, so importing the same
//! history twice gives the same Hebra commits. Commits that were exported by
//! Hebra (`Hebra-*` trailers) get their original change id and provenance
//! back, so a round trip through Git loses nothing.
//!
//! Authors keep their email and date, committers are recorded when they are
//! someone else, and every branch and tag in the stream comes along. Parents
//! outside the stream (an incremental fetch) are looked up in `known`.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::BufRead;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::{Error, Result};
use crate::gitodb::{self, Kind, Sha};
use crate::ids::{ChangeId, ObjectId};
use crate::model::{
    Actor, ActorKind, Commit, FileEntry, Provenance, StoredTree, TREE_VERSION, Tag, Tree,
};
use crate::repo::Tx;
use crate::store::{BLOB, TREE};

/// Flat trees kept in memory: parents are almost always recent, and anything
/// older is read back from the store. Keeping one per commit would cost
/// memory proportional to history times files.
const RECENT_TREES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ImportStats {
    pub commits: usize,
    pub blobs: usize,
    pub skipped_entries: usize,
}

/// What a stream contained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub stats: ImportStats,
    /// The commit of the last `commit` command, the tip for a single branch.
    pub tip: Option<ObjectId>,
    /// `refs/heads/...` and lightweight `refs/tags/...` as written in the
    /// stream, with their commits.
    pub refs: BTreeMap<String, ObjectId>,
    /// Annotated tags, by name.
    pub tags: BTreeMap<String, Tag>,
    /// Git commit id of each commit imported (when the stream has them).
    pub shas: Vec<(ObjectId, String)>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    name: String,
    email: Option<String>,
    seconds: i64,
}

/// Splits `Name <email> 1700000000 +0100`.
fn parse_identity(line: &[u8]) -> Identity {
    let text = String::from_utf8_lossy(line);
    let name = text.split('<').next().unwrap_or("").trim().to_owned();
    let email = text
        .split_once('<')
        .and_then(|(_, rest)| rest.split_once('>'))
        .map(|(email, _)| email.trim().to_owned())
        .filter(|e| !e.is_empty());
    let seconds = text
        .rsplit('>')
        .next()
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    Identity {
        name,
        email,
        seconds,
    }
}

fn rfc3339(seconds: i64) -> String {
    jiff::Timestamp::from_second(seconds)
        .map_or_else(|_| "1970-01-01T00:00:00Z".to_owned(), |t| t.to_string())
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

/// Flat trees of recent commits.
#[derive(Default)]
struct RecentTrees {
    order: VecDeque<ObjectId>,
    trees: HashMap<ObjectId, Tree>,
}

impl RecentTrees {
    fn insert(&mut self, id: ObjectId, tree: Tree) {
        if self.trees.insert(id, tree).is_none() {
            self.order.push_back(id);
        }
        if self.order.len() > RECENT_TREES
            && let Some(oldest) = self.order.pop_front()
        {
            self.trees.remove(&oldest);
        }
    }
}

fn parent_tree(tx: &mut Tx<'_>, recent: &RecentTrees, parent: &ObjectId) -> Result<Tree> {
    match recent.trees.get(parent) {
        Some(tree) => Ok(tree.clone()),
        None => tx.commit_tree(parent),
    }
}

/// Imports a fast-export stream. `known` finds the Hebra commit of a Git
/// commit imported earlier, for streams that continue existing history.
#[allow(clippy::too_many_lines)]
pub fn import(
    tx: &mut Tx<'_>,
    input: impl BufRead,
    known: &dyn Fn(&str) -> Option<ObjectId>,
) -> Result<Imported> {
    let mut p = Parser {
        input,
        peeked: None,
    };
    let mut blobs: HashMap<String, ObjectId> = HashMap::new();
    let mut commits: HashMap<String, ObjectId> = HashMap::new();
    let mut recent = RecentTrees::default();
    let mut out = Imported {
        stats: ImportStats {
            commits: 0,
            blobs: 0,
            skipped_entries: 0,
        },
        tip: None,
        refs: BTreeMap::new(),
        tags: BTreeMap::new(),
        shas: Vec::new(),
    };

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
                    out.stats.blobs += 1;
                    break;
                } else {
                    return Err(bad(format!("in blob: {}", String::from_utf8_lossy(&l))));
                }
            }
            continue;
        }
        if let Some(reference) = line.strip_prefix(b"reset ") {
            let reference = String::from_utf8_lossy(reference).into_owned();
            if p.peek()?.is_some_and(|l| l.starts_with(b"from ")) {
                let from = p.line()?.unwrap_or_default();
                let from = String::from_utf8_lossy(&from[5..]).into_owned();
                if let Some(id) = commits.get(&from).copied().or_else(|| known(&from)) {
                    out.refs.insert(reference, id);
                }
            }
            continue;
        }
        if let Some(name) = line.strip_prefix(b"tag ") {
            parse_tag(&mut p, name, &commits, known, &mut out)?;
            continue;
        }
        let Some(branch) = line.strip_prefix(b"commit ") else {
            return Err(bad(String::from_utf8_lossy(&line).into_owned()));
        };
        let branch = String::from_utf8_lossy(branch).into_owned();

        let mut mark = None;
        let mut original = None;
        let mut author = Identity {
            name: "unknown".to_owned(),
            email: None,
            seconds: 0,
        };
        let mut committer = None;
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
                committer = Some(parse_identity(c));
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
                let parent = commits
                    .get(&key)
                    .copied()
                    .or_else(|| known(&key))
                    .ok_or_else(|| bad(format!("unknown parent {key}")))?;
                if first {
                    tree = parent_tree(tx, &recent, &parent)?;
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
                        tree.files.insert(path, FileEntry::file(blob, executable));
                    }
                    b"120000" => {
                        let blob = *blobs
                            .get(&dataref)
                            .ok_or_else(|| bad(format!("unknown blob {dataref}")))?;
                        tree.files.insert(path, FileEntry::symlink(blob));
                    }
                    // A submodule: the "data" is the id of its commit.
                    b"160000" if dataref.len() == 40 => {
                        let blob = tx.store.write(BLOB, dataref.as_bytes())?;
                        tree.files.insert(path, FileEntry::gitlink(blob));
                    }
                    _ => out.stats.skipped_entries += 1,
                }
            } else if let Some(path) = l.strip_prefix(b"D ") {
                let path = unquote(path)?;
                // A file, or else a whole directory. Never both: Git deletes
                // a file `x` after adding `x/…` when a file becomes a folder.
                if tree.files.remove(&path).is_none() {
                    let prefix = format!("{path}/");
                    tree.files.retain(|p, _| !p.starts_with(&prefix));
                }
            } else if l.starts_with(b"commit ")
                || l.starts_with(b"reset ")
                || l.starts_with(b"tag ")
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
        let tree_id = tx.store.write_tree(&tree)?;
        let id = write_commit(
            tx,
            parents,
            tree_id,
            &message,
            &author,
            committer,
            original.as_deref(),
        )?;
        recent.insert(id, tree);
        if let Some(m) = mark {
            commits.insert(m, id);
        }
        if let Some(o) = &original {
            let sha = String::from_utf8_lossy(o).into_owned();
            commits.insert(sha.clone(), id);
            out.shas.push((id, sha));
        }
        out.stats.commits += 1;
        out.tip = Some(id);
        out.refs.insert(branch, id);
    }
    Ok(out)
}

/// Writes the Hebra commit for a Git commit.
fn write_commit(
    tx: &mut Tx<'_>,
    parents: Vec<ObjectId>,
    tree_id: ObjectId,
    message: &[u8],
    author: &Identity,
    committer: Option<Identity>,
    original: Option<&[u8]>,
) -> Result<ObjectId> {
    tx.write_commit(git_commit(
        parents, tree_id, message, author, committer, original,
    ))
}

/// The Hebra commit for a Git commit, the same whichever way it was read.
fn git_commit(
    parents: Vec<ObjectId>,
    tree_id: ObjectId,
    message: &[u8],
    author: &Identity,
    committer: Option<Identity>,
    original: Option<&[u8]>,
) -> Commit {
    let msg = parse_message(message);
    let change_id = msg
        .change_id
        .unwrap_or_else(|| ChangeId::derived_from(original.unwrap_or(message)));
    let actor = match msg.agent {
        Some(agent) => Actor {
            name: agent,
            kind: ActorKind::Agent,
            on_behalf_of: msg.on_behalf_of,
            email: author.email.clone(),
        },
        None => Actor {
            email: author.email.clone(),
            ..Actor::human(author.name.clone())
        },
    };
    let written = committer.as_ref().map_or(author.seconds, |c| c.seconds);
    let timestamp = rfc3339(if written != 0 {
        written
    } else {
        author.seconds
    });
    let mut commit = Commit::new(
        change_id,
        parents,
        tree_id,
        msg.description,
        actor,
        timestamp,
    );
    commit.provenance = msg.provenance;
    if written != author.seconds && author.seconds != 0 {
        commit.authored_at = Some(rfc3339(author.seconds));
    }
    if let Some(c) = committer
        && (c.name != author.name || c.email != author.email)
    {
        commit.committer = Some(Actor {
            email: c.email,
            ..Actor::human(c.name)
        });
    }
    commit
}

/// Delta chains are cut before the pack reader's limit.
const MAX_CHAIN: u8 = (crate::pack::MAX_DEPTH - 1) as u8;

#[derive(Debug, Clone, Copy)]
struct BlobInfo {
    id: ObjectId,
    depth: u8,
}

/// A Git blob delta as a Hebra delta: both objects gain the `blob\0`
/// prefix, so copies move by its length and the prefix is copied first.
fn carry_delta(git: &[u8], target_len: usize) -> Option<Vec<u8>> {
    let prefix = BLOB.len() + 1;
    let mut out = crate::delta::Writer::new(target_len + prefix);
    out.copy(0, prefix);
    gitodb::for_each_git_op(git, &mut |op| match op {
        gitodb::GitOp::Copy { offset, len } => out.copy(offset + prefix, len),
        gitodb::GitOp::Insert(bytes) => out.insert(bytes),
    })?;
    Some(out.finish())
}

/// The message as UTF-8, as `git fast-export --reencode=yes` gives it.
/// Latin-1 is converted; other encodings are read as UTF-8.
fn reencode(message: &[u8], encoding: Option<&[u8]>) -> Vec<u8> {
    let latin1 = encoding.is_some_and(|e| {
        let e = String::from_utf8_lossy(e).to_ascii_lowercase();
        matches!(
            e.as_str(),
            "iso-8859-1" | "iso8859-1" | "latin1" | "latin-1" | "l1" | "iso-8859-15"
        )
    });
    if latin1 && std::str::from_utf8(message).is_err() {
        return message
            .iter()
            .map(|&b| char::from(b))
            .collect::<String>()
            .into_bytes();
    }
    message.to_vec()
}

/// A tag message without its signature (`--signed-tags=strip`): from the
/// last line that starts one.
fn strip_signature(message: &[u8]) -> &[u8] {
    const MARKERS: [&[u8]; 4] = [
        b"-----BEGIN PGP SIGNATURE-----",
        b"-----BEGIN PGP MESSAGE-----",
        b"-----BEGIN SSH SIGNATURE-----",
        b"-----BEGIN SIGNED MESSAGE-----",
    ];
    let mut cut = message.len();
    let mut at = 0;
    while at < message.len() {
        if MARKERS.iter().any(|m| message[at..].starts_with(m)) {
            cut = at;
        }
        at = message[at..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(message.len(), |n| at + n + 1);
    }
    &message[..cut]
}

/// An object whose id is known, to be compressed and written with others
/// on every core: ids have to be worked out in history order, but writing
/// them does not.
struct Job {
    id: ObjectId,
    kind: &'static str,
    /// The whole object: kind, NUL, data.
    raw: Arc<Vec<u8>>,
    /// An earlier version to store it as a delta against.
    base: Option<(ObjectId, Arc<Vec<u8>>)>,
}

fn whole(kind: &str, data: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(kind.len() + 1 + data.len());
    raw.extend_from_slice(kind.as_bytes());
    raw.push(0);
    raw.extend_from_slice(data);
    raw
}

/// Commits converted between two parallel writes: bounds memory.
const JOB_COMMITS: usize = 2048;

fn write_jobs(store: &crate::store::Store, jobs: &[Job]) -> Result<()> {
    let next = AtomicUsize::new(0);
    let failed: Mutex<Option<Error>> = Mutex::new(None);
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get().min(16));
    let work = || {
        loop {
            let start = next.fetch_add(64, Ordering::Relaxed);
            let Some(chunk) = jobs.get(start..(start + 64).min(jobs.len())) else {
                return;
            };
            if chunk.is_empty() {
                return;
            }
            for job in chunk {
                if let Err(e) = write_job(store, job) {
                    if let Ok(mut f) = failed.lock() {
                        f.get_or_insert(e);
                    }
                    return;
                }
            }
        }
    };
    std::thread::scope(|scope| {
        for _ in 1..workers {
            scope.spawn(work);
        }
        work();
    });
    match failed.into_inner() {
        Ok(Some(e)) => Err(e),
        Ok(None) => Ok(()),
        Err(_) => Err(bad("a writer panicked")),
    }
}

fn write_job(store: &crate::store::Store, job: &Job) -> Result<()> {
    if let Some((base, base_raw)) = &job.base
        && let Some(delta) = crate::delta::encode(base_raw, &job.raw)
        && delta.len() * 2 < job.raw.len()
        && store.put_delta_pending(&job.id, base, &delta)?
    {
        return Ok(());
    }
    store.put_object(&job.id, job.kind, &job.raw[job.kind.len() + 1..])
}

/// Converts Git trees into Hebra directories, each once.
struct Trees<'a> {
    raw: &'a HashMap<Sha, (Kind, Vec<u8>)>,
    blobs: &'a HashMap<Sha, BlobInfo>,
    done: HashMap<Sha, Option<ObjectId>>,
    /// Directories already queued.
    queued: HashSet<ObjectId>,
    /// The last version at each directory path, to delta against, with
    /// the length of its chain.
    last: HashMap<String, (ObjectId, Arc<Vec<u8>>, u8)>,
    jobs: Vec<Job>,
}

impl Trees<'_> {
    fn convert(
        &mut self,
        tx: &Tx<'_>,
        sha: &Sha,
        path: &str,
        depth: usize,
    ) -> Result<Option<ObjectId>> {
        if let Some(done) = self.done.get(sha) {
            return Ok(*done);
        }
        if depth > 256 {
            return Err(bad("tree is too deep"));
        }
        let data = match self.raw.get(sha) {
            Some((Kind::Tree, data)) => data,
            _ => return Err(bad(format!("missing tree {}", gitodb::sha_hex(sha)))),
        };
        let entries = gitodb::tree_entries(data).ok_or_else(|| bad("corrupt tree"))?;
        let mut dir = StoredTree {
            v: TREE_VERSION,
            ..StoredTree::default()
        };
        for e in entries {
            let name = String::from_utf8(e.name.to_vec()).map_err(|_| bad("path is not UTF-8"))?;
            // Modes as Git reads them (`canon_mode`): anything that is not
            // a file, link or directory is a submodule.
            match e.mode & 0o170_000 {
                0o040_000 => {
                    let sub = format!("{path}{name}/");
                    if let Some(id) = self.convert(tx, &e.sha, &sub, depth + 1)? {
                        dir.dirs.insert(name, id);
                    }
                }
                0o100_000 | 0o120_000 => {
                    let blob = self
                        .blobs
                        .get(&e.sha)
                        .ok_or_else(|| bad(format!("unknown blob {}", gitodb::sha_hex(&e.sha))))?
                        .id;
                    let entry = if e.mode & 0o170_000 == 0o120_000 {
                        FileEntry::symlink(blob)
                    } else {
                        FileEntry::file(blob, e.mode & 0o100 != 0)
                    };
                    dir.files.insert(name, entry);
                }
                _ => {
                    let blob = tx.store.write(BLOB, gitodb::sha_hex(&e.sha).as_bytes())?;
                    dir.files.insert(name, FileEntry::gitlink(blob));
                }
            }
        }
        let id = if dir.files.is_empty() && dir.dirs.is_empty() {
            None
        } else {
            Some(self.queue(&dir, path)?)
        };
        self.done.insert(*sha, id);
        Ok(id)
    }

    fn queue(&mut self, dir: &StoredTree, path: &str) -> Result<ObjectId> {
        let bytes = serde_json::to_vec(dir)?;
        let id = ObjectId::hash(TREE, &bytes);
        if !self.queued.insert(id) {
            return Ok(id);
        }
        let raw = Arc::new(whole(TREE, &bytes));
        let (base, depth) = match self.last.get(path) {
            Some((base, base_raw, d)) if *d < MAX_CHAIN => {
                (Some((*base, Arc::clone(base_raw))), d + 1)
            }
            _ => (None, 0),
        };
        self.last
            .insert(path.to_owned(), (id, Arc::clone(&raw), depth));
        self.jobs.push(Job {
            id,
            kind: TREE,
            raw,
            base,
        });
        Ok(id)
    }
}

fn read_shallow(git_dir: &Path) -> HashSet<Sha> {
    std::fs::read_to_string(git_dir.join("shallow"))
        .unwrap_or_default()
        .lines()
        .filter_map(gitodb::parse_sha)
        .collect()
}

/// Imports the history behind `refs` (each a reference name and the
/// object it points at, a tag or a commit) straight from the objects of a
/// Git directory: the same commits [`import`] makes from the matching
/// `git fast-export --show-original-ids --signed-tags=strip --reencode=yes`
/// stream, in a fraction of the time. Every Git tree is converted once, and
/// Git's own deltas between file versions are kept. Call it inside
/// [`crate::store::Store::batch`].
#[allow(clippy::too_many_lines)]
pub fn import_objects(
    tx: &mut Tx<'_>,
    git_dir: &Path,
    refs: &[(String, String)],
    known: &dyn Fn(&str) -> Option<ObjectId>,
) -> Result<Imported> {
    let blobs: Mutex<HashMap<Sha, BlobInfo>> = Mutex::new(HashMap::new());
    let raw: Mutex<HashMap<Sha, (Kind, Vec<u8>)>> = Mutex::new(HashMap::new());
    let blob_count = AtomicUsize::new(0);
    let store = tx.store;
    let poisoned = || bad("import lock poisoned");
    gitodb::for_each_object(git_dir, &|object| {
        if object.kind != Kind::Blob {
            raw.lock()
                .map_err(|_| poisoned())?
                .insert(object.sha, (object.kind, object.data.to_vec()));
            return Ok(());
        }
        let id = ObjectId::hash(BLOB, object.data);
        let mut depth = 0;
        let mut stored = false;
        if let Some(git) = object.delta {
            let base = blobs
                .lock()
                .map_err(|_| poisoned())?
                .get(&git.base)
                .copied();
            if let Some(base) = base
                && base.depth < MAX_CHAIN
                && let Some(delta) = carry_delta(git.instructions, object.data.len())
                && store.put_delta(&id, &base.id, &delta)?
            {
                depth = base.depth + 1;
                stored = true;
            }
        }
        if !stored {
            store.put_object(&id, BLOB, object.data)?;
        }
        blob_count.fetch_add(1, Ordering::Relaxed);
        blobs
            .lock()
            .map_err(|_| poisoned())?
            .insert(object.sha, BlobInfo { id, depth });
        Ok(())
    })?;
    let blobs = blobs.into_inner().map_err(|_| poisoned())?;
    let raw = raw.into_inner().map_err(|_| poisoned())?;
    let shallow = read_shallow(git_dir);
    let commit_of = |sha: &Sha| match raw.get(sha) {
        Some((Kind::Commit, data)) => Some(data.as_slice()),
        _ => None,
    };

    // What each reference names: a commit, or a tag of one.
    let mut tips: Vec<(String, Sha, Option<Sha>)> = Vec::new();
    for (name, target) in refs {
        let Some(sha) = gitodb::parse_sha(target) else {
            continue;
        };
        match raw.get(&sha) {
            Some((Kind::Commit, _)) => tips.push((name.clone(), sha, None)),
            Some((Kind::Tag, data)) => {
                let h = gitodb::headers(data);
                if h.get(b"type") == Some(b"commit".as_slice())
                    && let Some(object) = h
                        .get(b"object")
                        .and_then(|o| gitodb::parse_sha(&String::from_utf8_lossy(o)))
                {
                    tips.push((name.clone(), object, Some(sha)));
                }
            }
            // Already imported, or not here at all.
            _ if known(target).is_some() => tips.push((name.clone(), sha, None)),
            _ => {}
        }
    }

    // Commits to convert, parents first.
    let parents_of = |data: &[u8]| -> Vec<Sha> {
        gitodb::headers(data)
            .all(b"parent")
            .filter_map(|p| gitodb::parse_sha(&String::from_utf8_lossy(p)))
            .collect()
    };
    let mut order: Vec<Sha> = Vec::new();
    let mut seen: HashSet<Sha> = HashSet::new();
    let mut stack: Vec<(Sha, bool)> = tips.iter().rev().map(|(_, sha, _)| (*sha, false)).collect();
    while let Some((sha, expanded)) = stack.pop() {
        if expanded {
            order.push(sha);
            continue;
        }
        if seen.contains(&sha) || known(&gitodb::sha_hex(&sha)).is_some() {
            continue;
        }
        let data = commit_of(&sha)
            .ok_or_else(|| bad(format!("missing commit {}", gitodb::sha_hex(&sha))))?;
        seen.insert(sha);
        stack.push((sha, true));
        if shallow.contains(&sha) {
            continue;
        }
        for parent in parents_of(data).into_iter().rev() {
            if !seen.contains(&parent) {
                stack.push((parent, false));
            }
        }
    }

    let mut out = Imported {
        stats: ImportStats {
            commits: 0,
            blobs: blob_count.into_inner(),
            skipped_entries: 0,
        },
        tip: None,
        refs: BTreeMap::new(),
        tags: BTreeMap::new(),
        shas: Vec::with_capacity(order.len()),
    };
    let mut converted: HashMap<Sha, ObjectId> = HashMap::with_capacity(order.len());
    let mut trees = Trees {
        raw: &raw,
        blobs: &blobs,
        done: HashMap::new(),
        queued: HashSet::new(),
        last: HashMap::new(),
        jobs: Vec::new(),
    };
    let unknown = Identity {
        name: "unknown".to_owned(),
        email: None,
        seconds: 0,
    };
    for (n, sha) in order.iter().enumerate() {
        if n % JOB_COMMITS == JOB_COMMITS - 1 {
            write_jobs(tx.store, &std::mem::take(&mut trees.jobs))?;
        }
        let data = commit_of(sha).ok_or_else(|| bad("missing commit"))?;
        let h = gitodb::headers(data);
        let hex = gitodb::sha_hex(sha);
        let tree_sha = h
            .get(b"tree")
            .and_then(|t| gitodb::parse_sha(&String::from_utf8_lossy(t)))
            .ok_or_else(|| bad(format!("commit {hex} has no tree")))?;
        let tree_id = match trees.convert(tx, &tree_sha, "", 0)? {
            Some(id) => id,
            None => tx.store.write_json(TREE, &Tree::default())?,
        };
        let mut parents = Vec::new();
        if !shallow.contains(sha) {
            for parent in parents_of(data) {
                let id = converted
                    .get(&parent)
                    .copied()
                    .or_else(|| known(&gitodb::sha_hex(&parent)))
                    .ok_or_else(|| bad(format!("unknown parent {}", gitodb::sha_hex(&parent))))?;
                parents.push(id);
            }
        }
        if parents.is_empty() {
            parents.push(tx.root_commit);
        }
        let author = h
            .get(b"author")
            .map_or_else(|| unknown.clone(), parse_identity);
        let committer = h.get(b"committer").map(parse_identity);
        let message = reencode(h.message, h.get(b"encoding"));
        let commit = git_commit(
            parents,
            tree_id,
            &message,
            &author,
            committer,
            Some(hex.as_bytes()),
        );
        let bytes = serde_json::to_vec(&commit)?;
        let id = ObjectId::hash(crate::store::COMMIT, &bytes);
        trees.jobs.push(Job {
            id,
            kind: crate::store::COMMIT,
            raw: Arc::new(whole(crate::store::COMMIT, &bytes)),
            base: None,
        });
        tx.remember_commit(id, commit);
        converted.insert(*sha, id);
        out.shas.push((id, hex));
        out.stats.commits += 1;
    }
    write_jobs(tx.store, &std::mem::take(&mut trees.jobs))?;

    let lookup = |sha: &Sha| {
        converted
            .get(sha)
            .copied()
            .or_else(|| known(&gitodb::sha_hex(sha)))
    };
    for (name, commit, tag) in &tips {
        let Some(id) = lookup(commit) else { continue };
        out.tip.get_or_insert(id);
        out.refs.insert(name.clone(), id);
        let (Some(tag), Some(short)) = (tag, name.strip_prefix("refs/tags/")) else {
            continue;
        };
        let Some((Kind::Tag, data)) = raw.get(tag) else {
            continue;
        };
        let h = gitodb::headers(data);
        let tagger = h.get(b"tagger").map(parse_identity);
        let message = String::from_utf8_lossy(strip_signature(h.message))
            .trim_end()
            .to_owned();
        out.tags.insert(
            short.to_owned(),
            Tag {
                target: id,
                message: Some(message),
                timestamp: tagger.as_ref().map(|t| rfc3339(t.seconds)),
                tagger: tagger.map(|t| Actor {
                    email: t.email,
                    ..Actor::human(t.name)
                }),
                signature: None,
            },
        );
    }
    Ok(out)
}

/// `tag <name>`, `mark`, `from`, `original-oid`, `tagger`, `data`.
fn parse_tag<R: BufRead>(
    p: &mut Parser<R>,
    name: &[u8],
    commits: &HashMap<String, ObjectId>,
    known: &dyn Fn(&str) -> Option<ObjectId>,
    out: &mut Imported,
) -> Result<()> {
    let name = String::from_utf8_lossy(name).into_owned();
    let (mut target, mut tagger) = (None, None);
    while let Some(l) = p.line()? {
        if let Some(from) = l.strip_prefix(b"from ") {
            let from = String::from_utf8_lossy(from).into_owned();
            target = commits.get(&from).copied().or_else(|| known(&from));
        } else if let Some(t) = l.strip_prefix(b"tagger ") {
            tagger = Some(parse_identity(t));
        } else if l.starts_with(b"data ") {
            let message = String::from_utf8_lossy(&p.data(&l)?).trim_end().to_owned();
            if let Some(target) = target {
                let timestamp = tagger.as_ref().map(|t: &Identity| rfc3339(t.seconds));
                out.tags.insert(
                    name,
                    Tag {
                        target,
                        message: Some(message),
                        tagger: tagger.map(|t| Actor {
                            email: t.email,
                            ..Actor::human(t.name)
                        }),
                        timestamp,
                        signature: None,
                    },
                );
            }
            break;
        }
    }
    Ok(())
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
    fn tag_signatures_are_stripped() {
        let msg =
            b"v1.0\n\nnotes\n-----BEGIN PGP SIGNATURE-----\nabc\n-----END PGP SIGNATURE-----\n";
        assert_eq!(strip_signature(msg), b"v1.0\n\nnotes\n");
        assert_eq!(strip_signature(b"plain\n"), b"plain\n");
    }

    #[test]
    fn git_deltas_carry_over() {
        // base "hello world" -> "hello there": copy 6, insert "there".
        let git = [11, 11, 0x90, 6, 5, b't', b'h', b'e', b'r', b'e'];
        let delta = carry_delta(&git, 11).unwrap();
        let out = crate::delta::apply(b"blob\0hello world", &delta).unwrap();
        assert_eq!(out, b"blob\0hello there");
    }

    #[test]
    fn parses_identities() {
        assert_eq!(
            parse_identity(b"Ana Lopez <ana@x.dev> 1700000000 +0100"),
            Identity {
                name: "Ana Lopez".to_owned(),
                email: Some("ana@x.dev".to_owned()),
                seconds: 1_700_000_000
            }
        );
        assert_eq!(parse_identity(b"Bot <> 5 +0000").email, None);
    }
}
