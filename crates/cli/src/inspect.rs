//! Looking at history: log, diff, show, blame, grep and the plumbing scripts
//! use. None of these change anything, so they never make anyone wait.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use anyhow::Result;
use hebra_core::blame;
use hebra_core::diff::{self, DiffOptions, FileDiff, FileStatus};
use hebra_core::history::PathSpec;
use hebra_core::model::{Commit, FileEntry, Tree};
use hebra_core::rename;
use hebra_core::store::{BLOB, COMMIT, TREE};
use hebra_core::wc::is_binary;
use hebra_core::{Error, ObjectId, Tx, Workspace};
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};

use crate::cli::{DiffArgs, GrepArgs, LogArgs, ShowArgs, StatusArgs};
use crate::commands::{Output, commit_json, short_line};
use crate::fmt::{ago, parse_date};
use crate::porcelain::Coded;

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidArgument(message.into())
}

fn pattern(text: &str, ignore_case: bool) -> Coded<Regex> {
    RegexBuilder::new(text)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|e| invalid(format!("invalid pattern: {e}")))
}

fn subject(c: &Commit) -> &str {
    c.description.lines().next().unwrap_or("")
}

fn body(c: &Commit) -> String {
    c.description
        .split_once("\n\n")
        .map(|(_, rest)| rest.to_owned())
        .unwrap_or_default()
}

/// A diff between a commit and what it started from, cheaply when possible.
fn commit_diff(tx: &mut Tx<'_>, id: &ObjectId, opts: DiffOptions) -> Coded<Vec<FileDiff>> {
    let commit = tx.commit(id)?;
    let (base, _) = tx.parents_tree(&commit.parents)?;
    let tree = tx.tree(&commit.tree)?;
    diff::diff_with(tx.store, &base, &tree, opts)
}

fn write_diff(out: &mut String, files: &[FileDiff], mode: &DiffMode) {
    match mode {
        DiffMode::None => {}
        DiffMode::NameOnly => {
            for f in files {
                let _ = writeln!(out, "{}", f.path);
            }
        }
        DiffMode::NameStatus => {
            for f in files {
                match &f.renamed_from {
                    Some(from) => {
                        let _ = writeln!(out, "R\t{from}\t{}", f.path);
                    }
                    None => {
                        let _ = writeln!(out, "{}\t{}", f.status.letter(), f.path);
                    }
                }
            }
        }
        DiffMode::Stat => {
            let width = files.iter().map(|f| f.path.len()).max().unwrap_or(0);
            let (mut added, mut removed) = (0, 0);
            for f in files {
                added += f.additions;
                removed += f.deletions;
                let detail = if f.binary {
                    "Bin".to_owned()
                } else {
                    format!(
                        "{} {}{}",
                        f.additions + f.deletions,
                        "+".repeat(f.additions.min(40)),
                        "-".repeat(f.deletions.min(40))
                    )
                };
                let _ = writeln!(out, " {:<width$} | {detail}", f.path);
            }
            if !files.is_empty() {
                let _ = writeln!(
                    out,
                    " {} file(s) changed, {added} insertion(s), {removed} deletion(s)",
                    files.len()
                );
            }
        }
        DiffMode::Patch => {
            for f in files {
                if f.binary {
                    let _ = writeln!(out, "Binary file {} changed", f.path);
                } else if let Some(patch) = &f.patch {
                    out.push_str(&diff::git_header(f));
                    out.push_str(patch);
                }
            }
        }
    }
}

enum DiffMode {
    None,
    Patch,
    Stat,
    NameOnly,
    NameStatus,
}

fn diff_mode(patch: bool, stat: bool, name_only: bool, name_status: bool) -> DiffMode {
    if name_only {
        DiffMode::NameOnly
    } else if name_status {
        DiffMode::NameStatus
    } else if stat {
        DiffMode::Stat
    } else if patch {
        DiffMode::Patch
    } else {
        DiffMode::None
    }
}

fn revision_or_path(tx: &mut Tx<'_>, arg: &str) -> Coded<Option<ObjectId>> {
    match tx.resolve(arg) {
        Ok(id) => Ok(Some(id)),
        Err(Error::RevisionNotFound(_) | Error::InvalidId(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn diff_cmd(tx: &mut Tx<'_>, args: &DiffArgs) -> Coded<Output> {
    let mut revisions = Vec::new();
    let mut paths = args.paths.clone();
    let mut range = None;
    for target in &args.targets {
        if let Some((a, b)) = target.split_once("...") {
            range = Some((a.to_owned(), b.to_owned(), true));
        } else if let Some((a, b)) = target.split_once("..") {
            range = Some((a.to_owned(), b.to_owned(), false));
        } else if let Some(id) = revision_or_path(tx, target)? {
            revisions.push(id);
        } else {
            paths.push(target.clone());
        }
    }
    let (from_tree, to_tree) = if let Some((a, b, symmetric)) = range {
        let a = tx.resolve(if a.is_empty() { "@" } else { &a })?;
        let b = tx.resolve(if b.is_empty() { "@" } else { &b })?;
        let from = if symmetric {
            tx.merge_base_tree(a, b)?
        } else {
            tx.commit_tree(&a)?
        };
        (from, tx.commit_tree(&b)?)
    } else if args.from.is_some() || args.to.is_some() {
        let from = tx.resolve(args.from.as_deref().unwrap_or("@-"))?;
        let to = tx.resolve(args.to.as_deref().unwrap_or("@"))?;
        (tx.commit_tree(&from)?, tx.commit_tree(&to)?)
    } else if let [a, b] = revisions[..] {
        (tx.commit_tree(&a)?, tx.commit_tree(&b)?)
    } else if let [a] = revisions[..] {
        let to = tx.wc_id()?;
        (tx.commit_tree(&a)?, tx.commit_tree(&to)?)
    } else {
        let id = tx.resolve(args.revision.as_deref().unwrap_or("@"))?;
        let commit = tx.commit(&id)?;
        (tx.parents_tree(&commit.parents)?.0, tx.tree(&commit.tree)?)
    };
    let spec = PathSpec::new(&paths)?;
    let mut files = diff::diff_with(
        tx.store,
        &from_tree,
        &to_tree,
        DiffOptions {
            with_patch: !(args.stat || args.name_only || args.name_status),
            context: args.context,
            renames: !args.no_renames,
        },
    )?;
    files.retain(|f| spec.matches(&f.path));
    let mut text = String::new();
    if files.is_empty() {
        let _ = writeln!(text, "No changes.");
    }
    write_diff(
        &mut text,
        &files,
        &diff_mode(true, args.stat, args.name_only, args.name_status),
    );
    Ok(Output::new(json!({ "files": files }), text).with_diff())
}

/// `--format=oneline`, and the other names Git gives to whole layouts.
fn format_template(format: Option<&String>, oneline: bool) -> Option<String> {
    match format.map(String::as_str) {
        Some("oneline") => Some("%h%d %s".to_owned()),
        Some("short" | "medium" | "full" | "fuller" | "raw") => None,
        Some(other) => Some(other.trim_start_matches("format:").to_owned()),
        None => oneline.then(|| "%h%d %s".to_owned()),
    }
}

/// Fills in a `--format` template.
fn render(template: &str, id: &ObjectId, c: &Commit, labels: &[String]) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    let (committer_name, committer_email) = match &c.committer {
        Some(who) => (who.name.clone(), who.email.clone().unwrap_or_default()),
        None => (
            c.author.name.clone(),
            c.author.email.clone().unwrap_or_default(),
        ),
    };
    let authored = c.authored_at.as_deref().unwrap_or(&c.timestamp);
    while let Some(ch) = chars.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('H') => out.push_str(&id.hex()),
            Some('h') => out.push_str(&id.short()),
            Some('s') => out.push_str(subject(c)),
            Some('b') => out.push_str(&body(c)),
            Some('B') => out.push_str(&c.description),
            Some('T') => out.push_str(&c.tree.hex()),
            Some('t') => out.push_str(&c.tree.short()),
            Some('P') => {
                let parents: Vec<String> = c.parents.iter().map(ObjectId::hex).collect();
                out.push_str(&parents.join(" "));
            }
            Some('p') => {
                let parents: Vec<String> = c.parents.iter().map(ObjectId::short).collect();
                out.push_str(&parents.join(" "));
            }
            Some('d') if !labels.is_empty() => {
                let _ = write!(out, " ({})", labels.join(", "));
            }
            Some('D') => out.push_str(&labels.join(", ")),
            Some('d') => {}
            Some('n') => out.push('\n'),
            Some('%') => out.push('%'),
            Some('a') => match chars.next() {
                Some('n') => out.push_str(&c.author.name),
                Some('e') => out.push_str(c.author.email.as_deref().unwrap_or("")),
                Some('d' | 'I' | 'i') => out.push_str(authored),
                Some('r') => out.push_str(&ago(authored)),
                Some(other) => {
                    let _ = write!(out, "%a{other}");
                }
                None => out.push_str("%a"),
            },
            Some('c') => match chars.next() {
                Some('n') => out.push_str(&committer_name),
                Some('e') => out.push_str(&committer_email),
                Some('d' | 'I' | 'i') => out.push_str(&c.timestamp),
                Some('r') => out.push_str(&ago(&c.timestamp)),
                Some(other) => {
                    let _ = write!(out, "%c{other}");
                }
                None => out.push_str("%c"),
            },
            Some(other) => {
                let _ = write!(out, "%{other}");
            }
            None => out.push('%'),
        }
    }
    out
}

/// Keeps the columns of `--graph`: each lane waits for one commit.
#[derive(Default)]
struct Graph {
    lanes: Vec<Option<ObjectId>>,
}

impl Graph {
    fn row(
        &mut self,
        id: &ObjectId,
        parents: &[ObjectId],
        symbol: char,
    ) -> (String, Option<String>) {
        let column = self
            .lanes
            .iter()
            .position(|l| l.as_ref() == Some(id))
            .or_else(|| self.lanes.iter().position(Option::is_none))
            .unwrap_or_else(|| {
                self.lanes.push(None);
                self.lanes.len() - 1
            });
        let row = self.line(|i| if i == column { symbol } else { '|' }, column);
        // Other lanes waiting for this same commit end here.
        for (i, lane) in self.lanes.iter_mut().enumerate() {
            if i != column && lane.as_ref() == Some(id) {
                *lane = None;
            }
        }
        self.lanes[column] = parents.first().copied();
        let mut merge = None;
        for parent in parents.iter().skip(1) {
            if self.lanes.iter().any(|l| l.as_ref() == Some(parent)) {
                continue;
            }
            let free = self.lanes.iter().position(Option::is_none);
            let lane = free.unwrap_or_else(|| {
                self.lanes.push(None);
                self.lanes.len() - 1
            });
            self.lanes[lane] = Some(*parent);
            merge = Some(self.line(|i| if i == lane { '\\' } else { '|' }, lane));
        }
        while self.lanes.last().is_some_and(Option::is_none) {
            self.lanes.pop();
        }
        (row, merge)
    }

    fn line(&self, cell: impl Fn(usize) -> char, column: usize) -> String {
        let width = self.lanes.len().max(column + 1);
        let mut out = String::new();
        for i in 0..width {
            let filled = i == column || self.lanes.get(i).is_some_and(Option::is_some);
            out.push(if filled { cell(i) } else { ' ' });
            out.push(' ');
        }
        out
    }

    fn padding(&self) -> String {
        let mut out = String::new();
        for lane in &self.lanes {
            out.push(if lane.is_some() { '|' } else { ' ' });
            out.push(' ');
        }
        out
    }
}

struct Filters {
    author: Option<Regex>,
    grep: Option<Regex>,
    line: Option<Regex>,
    pickaxe: Option<String>,
    since: Option<jiff::Timestamp>,
    until: Option<jiff::Timestamp>,
    merges: bool,
    no_merges: bool,
    spec: PathSpec,
    /// `--follow`: the path being followed, updated across renames.
    follow: Option<String>,
}

fn timestamp(text: &str) -> Coded<jiff::Timestamp> {
    parse_date(text)
        .map_err(|e| invalid(e.to_string()))?
        .parse()
        .map_err(|_| invalid(format!("invalid date `{text}`")))
}

impl Filters {
    fn new(args: &LogArgs, paths: &[String]) -> Coded<Self> {
        Ok(Self {
            author: args
                .author
                .as_deref()
                .map(|p| pattern(p, true))
                .transpose()?,
            grep: args
                .grep
                .as_deref()
                .map(|p| pattern(p, args.regexp_ignore_case))
                .transpose()?,
            line: args
                .line_regex
                .as_deref()
                .map(|p| pattern(p, false))
                .transpose()?,
            pickaxe: args.pickaxe.clone(),
            since: args.since.as_deref().map(timestamp).transpose()?,
            until: args.until.as_deref().map(timestamp).transpose()?,
            merges: args.merges,
            no_merges: args.no_merges,
            spec: PathSpec::new(paths)?,
            follow: if args.follow {
                paths.first().cloned()
            } else {
                None
            },
        })
    }

    fn accepts(&mut self, tx: &mut Tx<'_>, id: &ObjectId, c: &Commit) -> Coded<bool> {
        if (self.merges && c.parents.len() < 2) || (self.no_merges && c.parents.len() > 1) {
            return Ok(false);
        }
        if let Some(re) = &self.author {
            let email = c.author.email.as_deref().unwrap_or("");
            if !re.is_match(&c.author.name) && !re.is_match(email) {
                return Ok(false);
            }
        }
        if let Some(re) = &self.grep
            && !re.is_match(&c.description)
        {
            return Ok(false);
        }
        if self.since.is_some() || self.until.is_some() {
            let when: jiff::Timestamp = c.timestamp.parse().unwrap_or(jiff::Timestamp::UNIX_EPOCH);
            if self.since.is_some_and(|s| when < s) || self.until.is_some_and(|u| when > u) {
                return Ok(false);
            }
        }
        if let Some(path) = self.follow.clone() {
            return self.follows(tx, id, c, &path);
        }
        if !self.spec.is_all()
            && !tx
                .own_changes(id)?
                .iter()
                .any(|(p, _)| self.spec.matches(p))
        {
            return Ok(false);
        }
        if self.line.is_some() || self.pickaxe.is_some() {
            let files = commit_diff(
                tx,
                id,
                DiffOptions {
                    with_patch: true,
                    context: 0,
                    renames: false,
                },
            )?;
            return Ok(files.iter().any(|f| self.matches_diff(f)));
        }
        Ok(true)
    }

    fn matches_diff(&self, file: &FileDiff) -> bool {
        let Some(patch) = &file.patch else {
            return false;
        };
        let changed = patch.lines().filter(|l| {
            (l.starts_with('+') && !l.starts_with("+++"))
                || (l.starts_with('-') && !l.starts_with("---"))
        });
        if let Some(re) = &self.line {
            return changed.into_iter().any(|l| re.is_match(&l[1..]));
        }
        if let Some(text) = &self.pickaxe {
            let (mut added, mut removed) = (0usize, 0usize);
            for line in changed {
                let count = line[1..].matches(text.as_str()).count();
                if line.starts_with('+') {
                    added += count;
                } else {
                    removed += count;
                }
            }
            return added != removed;
        }
        false
    }

    /// Whether this commit touched the followed file, stepping to its old
    /// name when this is where it was renamed.
    fn follows(&mut self, tx: &mut Tx<'_>, id: &ObjectId, c: &Commit, path: &str) -> Coded<bool> {
        let changes = tx.own_changes(id)?;
        let Some((_, status)) = changes.iter().find(|(p, _)| p == path) else {
            return Ok(false);
        };
        if *status == FileStatus::Added
            && let Some(parent) = c.parents.first()
        {
            let before = tx.commit_tree(parent)?;
            let after = tx.tree(&c.tree)?;
            if let Some(r) = rename::detect(tx.store, &before, &after)?
                .into_iter()
                .find(|r| r.to == path)
            {
                self.follow = Some(r.from);
            }
        }
        Ok(true)
    }
}

/// The commits a list of revision arguments selects: `(tips, excluded)`.
fn selection(
    tx: &mut Tx<'_>,
    args: &[String],
    paths: &mut Vec<String>,
) -> Coded<(Vec<ObjectId>, HashSet<ObjectId>)> {
    let mut tips = Vec::new();
    let mut excluded = HashSet::new();
    for arg in args {
        if let Some((a, b)) = arg.split_once("...") {
            let a = tx.resolve(if a.is_empty() { "@" } else { a })?;
            let b = tx.resolve(if b.is_empty() { "@" } else { b })?;
            for base in tx.merge_bases(a, b)? {
                excluded.extend(tx.ancestors(base)?);
            }
            tips.extend([a, b]);
        } else if let Some((a, b)) = arg.split_once("..") {
            let a = tx.resolve(if a.is_empty() { "@" } else { a })?;
            let b = tx.resolve(if b.is_empty() { "@" } else { b })?;
            excluded.extend(tx.ancestors(a)?);
            tips.push(b);
        } else if let Some(rest) = arg.strip_prefix('^') {
            let id = tx.resolve(rest)?;
            excluded.extend(tx.ancestors(id)?);
        } else if let Some(id) = revision_or_path(tx, arg)? {
            tips.push(id);
        } else {
            paths.push(arg.clone());
        }
    }
    if tips.is_empty() {
        tips = tx.tips();
    }
    Ok((tips, excluded))
}

fn first_parent_chain(tx: &mut Tx<'_>, tips: &[ObjectId]) -> Coded<Vec<ObjectId>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for tip in tips {
        let mut current = Some(*tip);
        while let Some(id) = current {
            if !seen.insert(id) {
                break;
            }
            out.push(id);
            current = tx.node(&id)?.parents.first().copied();
        }
    }
    Ok(out)
}

fn labels(tx: &Tx<'_>) -> BTreeMap<ObjectId, Vec<String>> {
    let mut at: BTreeMap<ObjectId, Vec<String>> = BTreeMap::new();
    for (name, info) in &tx.view.workspaces {
        at.entry(info.wc).or_default().push(format!("{name}@"));
    }
    for (name, id) in &tx.view.bookmarks {
        at.entry(*id).or_default().push(name.clone());
    }
    for (remote, marks) in &tx.view.remotes {
        for (name, id) in marks {
            at.entry(*id).or_default().push(format!("{name}@{remote}"));
        }
    }
    for (name, tag) in &tx.view.tags {
        at.entry(tag.target)
            .or_default()
            .push(format!("tag: {name}"));
    }
    at
}

fn wants_details(args: &LogArgs) -> bool {
    args.patch || args.stat || args.name_only || args.name_status
}

#[allow(clippy::too_many_lines)]
pub fn log(tx: &mut Tx<'_>, args: &LogArgs) -> Coded<Output> {
    let mut paths = args.paths.clone();
    let (tips, excluded) = selection(tx, &args.revisions, &mut paths)?;
    let mut filters = Filters::new(args, &paths)?;
    let limit = if args.all && args.limit == 20 {
        usize::MAX
    } else {
        args.limit
    };
    let filtering = filters.author.is_some()
        || filters.grep.is_some()
        || filters.line.is_some()
        || filters.pickaxe.is_some()
        || filters.since.is_some()
        || filters.until.is_some()
        || filters.merges
        || filters.no_merges
        || !filters.spec.is_all()
        || !excluded.is_empty();
    let order = if args.first_parent {
        first_parent_chain(tx, &tips)?
    } else if filtering || args.reverse {
        tx.walk_from(&tips, usize::MAX)?
    } else {
        tx.walk_from(&tips, limit.saturating_add(1))?
    };
    let labels = labels(tx);
    let current = tx.wc_id()?;
    let template = format_template(args.format.as_ref(), args.oneline);
    let mut selected = Vec::new();
    let mut more = false;
    for id in order {
        if excluded.contains(&id) {
            continue;
        }
        let c = tx.commit(&id)?;
        if id == tx.root_commit && filtering {
            continue;
        }
        if !filters.accepts(tx, &id, &c)? {
            continue;
        }
        if selected.len() == limit {
            more = true;
            break;
        }
        selected.push((id, c));
    }
    if args.reverse {
        selected.reverse();
    }

    let mut text = String::new();
    let mut entries = Vec::new();
    let mut graph = Graph::default();
    let mode = diff_mode(args.patch, args.stat, args.name_only, args.name_status);
    for (id, c) in selected {
        let here = labels.get(&id).cloned().unwrap_or_default();
        let symbol = if id == current {
            '@'
        } else if id == tx.root_commit {
            '◆'
        } else if !c.conflicts.is_empty() {
            '×'
        } else {
            '*'
        };
        let (row, merge_row) = if args.graph {
            graph.row(&id, &c.parents, symbol)
        } else {
            (String::new(), None)
        };
        let pad = if args.graph {
            graph.padding()
        } else {
            String::new()
        };
        if let Some(template) = &template {
            let _ = writeln!(text, "{row}{}", render(template, &id, &c, &here));
        } else {
            let mark = if args.graph {
                row
            } else {
                format!("{}  ", if symbol == '*' { '○' } else { symbol })
            };
            let who = if id == tx.root_commit {
                "root()".to_owned()
            } else {
                c.author.display()
            };
            let when = if id == tx.root_commit {
                String::new()
            } else {
                ago(&c.timestamp)
            };
            let tags = if here.is_empty() {
                String::new()
            } else {
                format!("  [{}]", here.join(" "))
            };
            let merge_note = if c.parents.len() > 1 { "  (merge)" } else { "" };
            let _ = writeln!(
                text,
                "{mark}{} {}  {who}  {when}{tags}{merge_note}",
                c.change_id.short(),
                id.short()
            );
            if id != tx.root_commit {
                let summary = if tx.is_empty_change(&id)? {
                    "(empty)".to_owned()
                } else if c.description.trim().is_empty() {
                    "(no description)".to_owned()
                } else {
                    subject(&c).to_owned()
                };
                let bar = if args.graph { pad.as_str() } else { "│  " };
                let _ = writeln!(text, "{bar}{summary}");
                if let Some(task) = &c.provenance.task
                    && task != subject(&c)
                {
                    let _ = writeln!(text, "{bar}task: {task}");
                }
            }
        }
        if let Some(extra) = merge_row {
            let _ = writeln!(text, "{extra}");
        }
        let mut entry = commit_json(&id, &c);
        entry["labels"] = json!(here);
        entry["current"] = json!(id == current);
        entry["empty"] = json!(id != tx.root_commit && tx.is_empty_change(&id)?);
        if wants_details(args) {
            let files = commit_diff(
                tx,
                &id,
                DiffOptions {
                    with_patch: args.patch,
                    ..DiffOptions::default()
                },
            )?;
            write_diff(&mut text, &files, &mode);
            entry["files"] = json!(files);
        }
        entries.push(entry);
    }
    if more && template.is_none() {
        let _ = writeln!(text, "… more (use -n)");
    }
    Ok(Output::new(json!({ "commits": entries, "more": more }), text).with_diff())
}

pub fn show(tx: &mut Tx<'_>, args: &ShowArgs) -> Coded<Output> {
    let revision = match &args.revision {
        Some(rev) => rev.clone(),
        None => {
            let wc = tx.wc_id()?;
            if tx.is_empty_change(&wc)? { "@-" } else { "@" }.to_owned()
        }
    };
    if let Some((rev, path)) = revision.split_once(':') {
        return file_at(tx, rev, path);
    }
    let id = tx.resolve(&revision)?;
    let c = tx.commit(&id)?;
    let files = commit_diff(
        tx,
        &id,
        DiffOptions {
            with_patch: !args.stat && !args.name_only && !args.name_status,
            ..DiffOptions::default()
        },
    )?;
    let mut text = String::new();
    if let Some(template) = format_template(args.format.as_ref(), args.oneline) {
        let here = labels(tx).remove(&id).unwrap_or_default();
        let _ = writeln!(text, "{}", render(&template, &id, &c, &here));
    } else {
        let _ = writeln!(text, "Change:  {}", c.change_id);
        let _ = writeln!(text, "Commit:  {id}");
        let author = match &c.author.email {
            Some(email) => format!("{} <{email}>", c.author.display()),
            None => c.author.display(),
        };
        let _ = writeln!(text, "Author:  {author}");
        if let Some(committer) = &c.committer {
            let _ = writeln!(text, "Commit by: {}", committer.display());
        }
        let _ = writeln!(text, "Date:    {} ({})", c.timestamp, ago(&c.timestamp));
        for p in &c.parents {
            let _ = writeln!(text, "Parent:  {}", p.short());
        }
        if let Some(task) = &c.provenance.task {
            let _ = writeln!(text, "Task:    {task}");
        }
        if let Some(session) = &c.provenance.session {
            let _ = writeln!(text, "Session: {session}");
        }
        if let Some(prompt) = &c.provenance.prompt {
            let _ = writeln!(text, "Prompt:  {prompt}");
        }
        if let Some(note) = tx.note_get(&id)? {
            let _ = writeln!(text, "Note:    {note}");
        }
        let description = if c.description.trim().is_empty() {
            "(no description)".to_owned()
        } else {
            c.description.replace('\n', "\n    ")
        };
        let _ = writeln!(text, "\n    {description}\n");
        if !c.conflicts.is_empty() {
            let _ = writeln!(text, "Conflicts (recorded, not blocking):");
            for conflict in &c.conflicts {
                let _ = writeln!(text, "  ! {}", conflict.path);
            }
        }
    }
    if !args.no_patch {
        write_diff(
            &mut text,
            &files,
            &diff_mode(true, args.stat, args.name_only, args.name_status),
        );
    }
    let mut json = commit_json(&id, &c);
    json["files"] = json!(files);
    Ok(Output::new(json, text).with_diff())
}

fn file_at(tx: &mut Tx<'_>, rev: &str, path: &str) -> Coded<Output> {
    let id = tx.resolve(if rev.is_empty() { "@" } else { rev })?;
    let tree = tx.commit_tree(&id)?;
    let entry = tree
        .files
        .get(path)
        .ok_or_else(|| invalid(format!("`{path}` does not exist in {}", id.short())))?;
    let data = tx.store.read(BLOB, &entry.blob)?;
    let text = String::from_utf8_lossy(&data).into_owned();
    Ok(Output::binary(
        json!({ "path": path, "commit": id.hex(), "blob": entry.blob.hex(), "content": text }),
        data,
    ))
}

pub fn status_short(tx: &mut Tx<'_>, args: &StatusArgs) -> Coded<Output> {
    let wc = tx.wc_id()?;
    let commit = tx.commit(&wc)?;
    let mut text = String::new();
    if args.branch {
        let branch = tx
            .current_bookmark()
            .unwrap_or_else(|| "HEAD (no branch)".to_owned());
        let _ = writeln!(text, "## {branch}");
    }
    let (base, _) = tx.parents_tree(&commit.parents)?;
    let tree = tx.tree(&commit.tree)?;
    let files = diff::diff_with(
        tx.store,
        &base,
        &tree,
        DiffOptions {
            with_patch: false,
            ..DiffOptions::default()
        },
    )?;
    let conflicted: HashSet<&str> = commit.conflicts.iter().map(|c| c.path.as_str()).collect();
    let mut rows = Vec::new();
    for f in &files {
        let code = if conflicted.contains(f.path.as_str()) {
            "UU"
        } else {
            match f.status {
                FileStatus::Added => "A ",
                FileStatus::Modified => " M",
                FileStatus::Deleted => " D",
                FileStatus::Renamed => "R ",
            }
        };
        match &f.renamed_from {
            Some(from) => {
                let _ = writeln!(text, "{code} {from} -> {}", f.path);
            }
            None => {
                let _ = writeln!(text, "{code} {}", f.path);
            }
        }
        rows.push(json!({ "path": f.path, "status": f.status, "renamed_from": f.renamed_from }));
    }
    Ok(Output::new(json!({ "changes": rows }), text))
}

pub fn blame_cmd(
    tx: &mut Tx<'_>,
    path: &str,
    revision: &str,
    lines: Option<&str>,
) -> Coded<Output> {
    let start = tx.resolve(revision)?;
    let range = lines
        .map(|text| -> Coded<(usize, usize)> {
            let (a, b) = text.split_once(',').unwrap_or((text, text));
            let parse = |s: &str| {
                s.trim()
                    .parse::<usize>()
                    .map_err(|_| invalid(format!("invalid line range `{text}`")))
            };
            let first = parse(a)?;
            // `10,+5` is five lines from the tenth, as in Git.
            let last = match b.strip_prefix('+') {
                Some(count) => first + parse(count)?,
                None => parse(b)?,
            };
            Ok((first, last))
        })
        .transpose()?;
    let attribution = blame::blame(tx, start, path)?;
    let width = attribution.len().to_string().len();
    let mut text = String::new();
    let mut rows = Vec::new();
    for line in &attribution {
        if range.is_some_and(|(a, b)| line.line < a || line.line > b) {
            continue;
        }
        let c = tx.commit(&line.commit)?;
        let date = c.timestamp.get(..10).unwrap_or(&c.timestamp);
        let _ = writeln!(
            text,
            "{} ({} {date} {:>width$}) {}",
            line.commit.short(),
            c.author.display(),
            line.line,
            line.text
        );
        rows.push(json!({
            "line": line.line,
            "text": line.text,
            "path": line.path,
            "commit": line.commit.hex(),
            "change_id": c.change_id.encode(),
            "author": c.author,
            "timestamp": c.timestamp,
            "provenance": c.provenance,
            "description": subject(&c),
        }));
    }
    Ok(Output::new(json!({ "path": path, "lines": rows }), text))
}

pub fn grep(tx: &mut Tx<'_>, args: &GrepArgs) -> Coded<Output> {
    let id = tx.resolve(args.revision.as_deref().unwrap_or("@"))?;
    let tree = tx.commit_tree(&id)?;
    let source = if args.fixed_strings {
        regex::escape(&args.pattern)
    } else {
        args.pattern.clone()
    };
    let source = if args.word_regexp {
        format!(r"\b(?:{source})\b")
    } else {
        source
    };
    let re = pattern(&source, args.ignore_case)?;
    let spec = PathSpec::new(&args.paths)?;
    let mut text = String::new();
    let mut hits = Vec::new();
    let mut total = 0;
    for (path, entry) in &tree.files {
        if !entry.is_file() || !spec.matches(path) {
            continue;
        }
        let data = tx.store.read(BLOB, &entry.blob)?;
        if is_binary(&data) {
            continue;
        }
        let content = String::from_utf8_lossy(&data);
        let mut count = 0;
        for (n, line) in content.lines().enumerate() {
            if re.is_match(line) == args.invert_match {
                continue;
            }
            count += 1;
            total += 1;
            if !args.files_with_matches && !args.count {
                let _ = writeln!(text, "{path}:{}:{line}", n + 1);
                hits.push(json!({ "path": path, "line": n + 1, "text": line }));
            }
        }
        if count > 0 {
            if args.files_with_matches {
                let _ = writeln!(text, "{path}");
                hits.push(json!({ "path": path }));
            } else if args.count {
                let _ = writeln!(text, "{path}:{count}");
                hits.push(json!({ "path": path, "count": count }));
            }
        }
    }
    Ok(Output::with_exit(
        json!({ "matches": hits, "total": total }),
        text,
        u8::from(total == 0),
    ))
}

pub fn shortlog(tx: &mut Tx<'_>, summary: bool, email: bool, revision: &str) -> Coded<Output> {
    let tip = tx.resolve(revision)?;
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for id in tx.walk_from(&[tip], usize::MAX)? {
        if id == tx.root_commit {
            continue;
        }
        let c = tx.commit(&id)?;
        let name = match (&c.author.email, email) {
            (Some(address), true) => format!("{} <{address}>", c.author.name),
            _ => c.author.name.clone(),
        };
        by.entry(name).or_default().push(subject(&c).to_owned());
    }
    let mut rows: Vec<(String, Vec<String>)> = by.into_iter().collect();
    rows.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    let mut text = String::new();
    for (name, subjects) in &rows {
        if summary {
            let _ = writeln!(text, "{:>6}\t{name}", subjects.len());
        } else {
            let _ = writeln!(text, "{name} ({}):", subjects.len());
            for s in subjects {
                let _ = writeln!(text, "      {s}");
            }
            let _ = writeln!(text);
        }
    }
    let json: Vec<Value> = rows
        .iter()
        .map(|(n, s)| json!({ "author": n, "commits": s.len() }))
        .collect();
    Ok(Output::new(json!({ "authors": json }), text))
}

pub fn ls_files(tx: &mut Tx<'_>, modified: bool, deleted: bool, paths: &[String]) -> Coded<Output> {
    let spec = PathSpec::new(paths)?;
    let wc = tx.wc_id()?;
    let names: Vec<String> = if modified || deleted {
        tx.own_changes(&wc)?
            .into_iter()
            .filter(|(_, s)| {
                (modified && *s == FileStatus::Modified) || (deleted && *s == FileStatus::Deleted)
            })
            .map(|(p, _)| p)
            .filter(|p| spec.matches(p))
            .collect()
    } else {
        tx.commit_tree(&wc)?
            .files
            .into_keys()
            .filter(|p| spec.matches(p))
            .collect()
    };
    let text: String = names.iter().map(|p| format!("{p}\n")).collect();
    Ok(Output::new(json!({ "files": names }), text))
}

fn mode_of(entry: &FileEntry) -> &'static str {
    if entry.gitlink {
        "160000"
    } else if entry.symlink {
        "120000"
    } else if entry.executable {
        "100755"
    } else {
        "100644"
    }
}

pub fn ls_tree(
    tx: &mut Tx<'_>,
    revision: &str,
    recursive: bool,
    name_only: bool,
    path: Option<&str>,
) -> Coded<Output> {
    let id = tx.resolve(revision)?;
    let tree: Tree = tx.commit_tree(&id)?;
    let prefix = path
        .map(|p| format!("{}/", p.trim_end_matches('/')))
        .unwrap_or_default();
    let mut rows = Vec::new();
    let mut text = String::new();
    let mut dirs = std::collections::BTreeSet::new();
    for (file, entry) in &tree.files {
        let Some(rest) = file.strip_prefix(&prefix) else {
            continue;
        };
        if !recursive && let Some((dir, _)) = rest.split_once('/') {
            if dirs.insert(dir.to_owned()) {
                let name = format!("{prefix}{dir}");
                let _ = writeln!(
                    text,
                    "{}",
                    if name_only {
                        name.clone()
                    } else {
                        format!("040000 tree -\t{name}")
                    }
                );
                rows.push(json!({ "mode": "040000", "kind": "tree", "path": name }));
            }
            continue;
        }
        let mode = mode_of(entry);
        let _ = writeln!(
            text,
            "{}",
            if name_only {
                file.clone()
            } else {
                format!("{mode} blob {}\t{file}", entry.blob.short())
            }
        );
        rows.push(json!({ "mode": mode, "kind": "blob", "blob": entry.blob.hex(), "path": file }));
    }
    Ok(Output::new(json!({ "entries": rows }), text))
}

pub fn cat(tx: &mut Tx<'_>, spec: &str, kind: bool) -> Coded<Output> {
    if let Some((rev, path)) = spec.split_once(':')
        && !kind
    {
        return file_at(tx, rev, path);
    }
    let raw_id = ObjectId::parse(spec).ok().filter(|id| tx.store.has(id));
    let id = match raw_id {
        Some(id) => id,
        None => tx.resolve(spec)?,
    };
    let raw = tx.store.read_raw(&id)?;
    let nul = raw.iter().position(|&b| b == 0).unwrap_or(0);
    let found = String::from_utf8_lossy(&raw[..nul]).into_owned();
    if kind {
        return Ok(Output::new(
            json!({ "id": id.hex(), "kind": found }),
            format!("{found}\n"),
        ));
    }
    let content = raw[nul + 1..].to_vec();
    if found == COMMIT || found == TREE {
        let pretty = serde_json::from_slice::<Value>(&content)
            .and_then(|v| serde_json::to_string_pretty(&v))
            .unwrap_or_default();
        return Ok(Output::new(
            json!({ "id": id.hex(), "kind": found, "content": pretty }),
            format!("{pretty}\n"),
        ));
    }
    Ok(Output::binary(
        json!({ "id": id.hex(), "kind": found }),
        content,
    ))
}

pub fn rev_parse(
    tx: &mut Tx<'_>,
    revisions: &[String],
    abbrev_ref: bool,
    short: bool,
) -> Coded<Output> {
    let mut lines = Vec::new();
    for rev in revisions {
        if abbrev_ref && matches!(rev.as_str(), "HEAD" | "@") {
            lines.push(tx.current_bookmark().unwrap_or_else(|| "HEAD".to_owned()));
            continue;
        }
        let id = tx.resolve(rev)?;
        lines.push(if short { id.short() } else { id.hex() });
    }
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    Ok(Output::new(json!({ "revisions": lines }), text))
}

pub fn rev_list(
    tx: &mut Tx<'_>,
    revisions: &[String],
    count: bool,
    limit: Option<usize>,
) -> Coded<Output> {
    let mut paths = Vec::new();
    let (tips, excluded) = selection(tx, revisions, &mut paths)?;
    let mut ids: Vec<ObjectId> = tx
        .walk_from(&tips, usize::MAX)?
        .into_iter()
        .filter(|id| !excluded.contains(id) && *id != tx.root_commit)
        .collect();
    if let Some(n) = limit {
        ids.truncate(n);
    }
    if count {
        return Ok(Output::new(
            json!({ "count": ids.len() }),
            format!("{}\n", ids.len()),
        ));
    }
    let text: String = ids.iter().map(|id| format!("{id}\n")).collect();
    Ok(Output::new(
        json!({ "commits": ids.iter().map(ObjectId::hex).collect::<Vec<_>>() }),
        text,
    ))
}

pub fn merge_base(
    tx: &mut Tx<'_>,
    a: &str,
    b: &str,
    is_ancestor: bool,
    all: bool,
) -> Coded<Output> {
    let (a, b) = (tx.resolve(a)?, tx.resolve(b)?);
    if is_ancestor {
        let yes = tx.is_ancestor(a, b)?;
        return Ok(Output::with_exit(
            json!({ "is_ancestor": yes }),
            String::new(),
            u8::from(!yes),
        ));
    }
    let bases = tx.merge_bases(a, b)?;
    let shown = if all {
        &bases[..]
    } else {
        &bases[..bases.len().min(1)]
    };
    let text: String = shown.iter().map(|id| format!("{id}\n")).collect();
    Ok(Output::with_exit(
        json!({ "bases": shown.iter().map(ObjectId::hex).collect::<Vec<_>>() }),
        text,
        u8::from(shown.is_empty()),
    ))
}

pub fn for_each_ref(tx: &Tx<'_>, pattern: Option<&str>) -> Coded<Output> {
    let mut rows: Vec<(String, ObjectId)> = Vec::new();
    for (name, id) in &tx.view.bookmarks {
        rows.push((format!("refs/heads/{name}"), *id));
    }
    for (remote, marks) in &tx.view.remotes {
        for (name, id) in marks {
            rows.push((format!("refs/remotes/{remote}/{name}"), *id));
        }
    }
    for (name, tag) in &tx.view.tags {
        rows.push((format!("refs/tags/{name}"), tag.target));
    }
    rows.retain(|(name, _)| pattern.is_none_or(|p| name.starts_with(p)));
    let mut text = String::new();
    for (name, id) in &rows {
        let _ = writeln!(text, "{id} commit\t{name}");
    }
    let json: Vec<Value> = rows
        .iter()
        .map(|(n, id)| json!({ "ref": n, "commit": id.hex() }))
        .collect();
    Ok(Output::new(json!({ "refs": json }), text))
}

pub fn name_rev(tx: &mut Tx<'_>, revision: &str) -> Coded<Output> {
    let id = tx.resolve(revision)?;
    let mut best: Option<(usize, String)> = None;
    let mut names: Vec<(String, ObjectId)> = tx
        .view
        .tags
        .iter()
        .map(|(n, t)| (format!("tags/{n}"), t.target))
        .collect();
    names.extend(tx.view.bookmarks.iter().map(|(n, i)| (n.clone(), *i)));
    for (name, tip) in names {
        if tx.is_ancestor(id, tip)? {
            let distance = tx.range(Some(id), tip)?.len();
            if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                best = Some((distance, name));
            }
        }
    }
    let name = match best {
        Some((0, name)) => name,
        Some((n, name)) => format!("{name}~{n}"),
        None => "undefined".to_owned(),
    };
    Ok(Output::new(
        json!({ "commit": id.hex(), "name": name }),
        format!("{} {name}\n", id.short()),
    ))
}

/// What a commit changes, ignoring where: two commits with the same one are
/// the same patch (Git's patch id).
fn patch_id(tx: &mut Tx<'_>, id: &ObjectId) -> Coded<String> {
    let files = commit_diff(
        tx,
        id,
        DiffOptions {
            with_patch: true,
            context: 3,
            renames: false,
        },
    )?;
    let mut hasher = String::new();
    for f in files {
        hasher.push_str(&f.path);
        for line in f.patch.unwrap_or_default().lines() {
            if !line.starts_with("@@") {
                hasher.push_str(line);
                hasher.push('\n');
            }
        }
    }
    Ok(ObjectId::hash("patch", hasher.as_bytes()).hex())
}

pub fn cherry(tx: &mut Tx<'_>, upstream: &str, head: &str) -> Coded<Output> {
    let (up, tip) = (tx.resolve(upstream)?, tx.resolve(head)?);
    let base = tx.common_ancestor(up, tip)?;
    let mut applied = HashSet::new();
    for id in tx.range(Some(base), up)? {
        applied.insert(patch_id(tx, &id)?);
    }
    let mut text = String::new();
    let mut rows = Vec::new();
    for id in tx.range(Some(base), tip)? {
        let done = applied.contains(&patch_id(tx, &id)?);
        let _ = writeln!(text, "{} {id}", if done { '-' } else { '+' });
        rows.push(json!({ "commit": id.hex(), "applied": done }));
    }
    Ok(Output::new(json!({ "commits": rows }), text))
}

pub fn range_diff(tx: &mut Tx<'_>, old: &str, new: &str) -> Coded<Output> {
    let mut expand = |spec: &str| -> Coded<Vec<(ObjectId, Commit, String)>> {
        let mut out = Vec::new();
        for id in crate::porcelain::expand_range(tx, spec)? {
            let commit = tx.commit(&id)?;
            let patch = patch_id(tx, &id)?;
            out.push((id, commit, patch));
        }
        Ok(out)
    };
    let before = expand(old)?;
    let after = expand(new)?;
    let mut text = String::new();
    let mut rows = Vec::new();
    for (id, commit, patch) in &after {
        let match_ = before
            .iter()
            .find(|(_, c, _)| c.change_id == commit.change_id);
        let status = match match_ {
            Some((_, _, old_patch)) if old_patch == patch => "=",
            Some(_) => "!",
            None => ">",
        };
        let _ = writeln!(text, "{status} {} {}", id.short(), subject(commit));
        rows.push(json!({ "commit": id.hex(), "status": status }));
    }
    for (id, commit, _) in &before {
        if !after
            .iter()
            .any(|(_, c, _)| c.change_id == commit.change_id)
        {
            let _ = writeln!(text, "< {} {}", id.short(), subject(commit));
            rows.push(json!({ "commit": id.hex(), "status": "<" }));
        }
    }
    Ok(Output::new(json!({ "changes": rows }), text))
}

pub fn show_branch(tx: &mut Tx<'_>, revisions: &[String]) -> Coded<Output> {
    let names: Vec<String> = if revisions.is_empty() {
        tx.view.bookmarks.keys().cloned().collect()
    } else {
        revisions.to_vec()
    };
    let mut tips = Vec::new();
    for name in &names {
        tips.push(tx.resolve(name)?);
    }
    let mut text = String::new();
    for (i, name) in names.iter().enumerate() {
        let c = tx.commit(&tips[i])?;
        let _ = writeln!(text, "{}* [{name}] {}", " ".repeat(i), subject(&c));
    }
    let _ = writeln!(text, "{}", "-".repeat(names.len().max(1)));
    let mut rows = Vec::new();
    for id in tx.walk_from(&tips, 32)? {
        if id == tx.root_commit {
            continue;
        }
        let mut marks = String::new();
        for tip in &tips {
            marks.push(if tx.is_ancestor(id, *tip)? { '+' } else { ' ' });
        }
        let c = tx.commit(&id)?;
        let _ = writeln!(text, "{marks} {}", short_line(&id, &c));
        rows.push(json!({ "commit": id.hex(), "in": marks }));
    }
    Ok(Output::new(
        json!({ "branches": names, "commits": rows }),
        text,
    ))
}

/// Prints the operation's changes to the repository.
pub fn op_show(ws: &Workspace, id: i64) -> Result<Output> {
    let (op, delta) = ws.operation_delta(id)?;
    let json = json!({ "operation": op, "changes": delta });
    let text = serde_json::to_string_pretty(&delta).unwrap_or_default();
    Ok(Output::new(
        json,
        format!("{} ({})\n{text}\n", op.command, op.time),
    ))
}
