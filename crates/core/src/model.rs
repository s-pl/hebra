//! The data model: trees, commits, actors and the repository view.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ids::{ChangeId, ObjectId};

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub blob: ObjectId,
    #[serde(default, skip_serializing_if = "is_false")]
    pub executable: bool,
    /// A symbolic link; the blob holds its target, with `/` separators.
    #[serde(default, skip_serializing_if = "is_false")]
    pub symlink: bool,
    /// A Git submodule; the blob holds the 40-hex id of its commit.
    #[serde(default, skip_serializing_if = "is_false")]
    pub gitlink: bool,
}

impl FileEntry {
    /// A regular file.
    #[must_use]
    pub fn file(blob: ObjectId, executable: bool) -> Self {
        Self {
            blob,
            executable,
            symlink: false,
            gitlink: false,
        }
    }

    /// A symbolic link whose target is stored in `blob`.
    #[must_use]
    pub fn symlink(blob: ObjectId) -> Self {
        Self {
            blob,
            executable: false,
            symlink: true,
            gitlink: false,
        }
    }

    /// A Git submodule pinned at the commit whose id is stored in `blob`.
    #[must_use]
    pub fn gitlink(blob: ObjectId) -> Self {
        Self {
            blob,
            executable: false,
            symlink: false,
            gitlink: true,
        }
    }

    /// A regular file, with content to show, diff and merge.
    #[must_use]
    pub fn is_file(&self) -> bool {
        !self.symlink && !self.gitlink
    }
}

/// A snapshot of files, keyed by `/`-separated path relative to the
/// workspace root. In memory it is flat; the store keeps it as a Merkle tree
/// with one object per directory (see [`crate::store::Store::write_tree`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    pub files: BTreeMap<String, FileEntry>,
}

/// A tree object as stored. Version 0 (no `v`) is the original flat format:
/// `files` holds full paths. Version 2 is one directory: `files` holds the
/// names of its files and `dirs` the tree objects of its subdirectories.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTree {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub v: u32,
    #[serde(default)]
    pub files: BTreeMap<String, FileEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dirs: BTreeMap<String, ObjectId>,
}

/// Version of [`StoredTree`] written for directories.
pub const TREE_VERSION: u32 = 2;

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(v: &u32) -> bool {
    *v == 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    Human,
    Agent,
}

/// Who did something: a person, or an agent acting for a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub name: String,
    pub kind: ActorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_behalf_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

impl Actor {
    #[must_use]
    pub fn human(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: ActorKind::Human,
            on_behalf_of: None,
            email: None,
        }
    }

    /// `ana`, or `claude-code (agent for ana)`.
    #[must_use]
    pub fn display(&self) -> String {
        match (&self.kind, &self.on_behalf_of) {
            (ActorKind::Agent, Some(person)) => format!("{} (agent for {person})", self.name),
            (ActorKind::Agent, None) => format!("{} (agent)", self.name),
            (ActorKind::Human, _) => self.name.clone(),
        }
    }
}

/// Why a change exists: the task, agent session and prompt behind it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

impl Provenance {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.task.is_none() && self.session.is_none() && self.prompt.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// Both sides edited the same lines; the file holds conflict markers.
    Content,
    /// One side modified the file and the other deleted it.
    ModifyDelete,
    /// Both sides changed a binary file, a symlink or a submodule; ours was
    /// kept.
    Binary,
    /// Both sides renamed the file, to different names. Both were kept.
    Rename,
}

/// A conflict recorded inside a commit. Conflicts never block work: they are
/// committed like any other state and resolved later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub path: String,
    pub kind: ConflictKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub change_id: ChangeId,
    pub parents: Vec<ObjectId>,
    /// Earlier versions of this same change (set when a commit is rewritten).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub predecessors: Vec<ObjectId>,
    pub tree: ObjectId,
    pub description: String,
    pub author: Actor,
    /// RFC 3339 timestamp of when this version was written.
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Provenance::is_empty")]
    pub provenance: Provenance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<Conflict>,
    /// When the work was first written, if that differs from `timestamp`
    /// (Git's author date).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authored_at: Option<String>,
    /// Who wrote this version, when that is not the author (Git's committer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committer: Option<Actor>,
    /// A detached signature over this commit without this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Set on the commit that carries an annotated tag to a server: it names
    /// the tag, its only parent is the target and its description the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

impl Commit {
    /// A commit with only what every commit needs.
    #[must_use]
    pub fn new(
        change_id: ChangeId,
        parents: Vec<ObjectId>,
        tree: ObjectId,
        description: String,
        author: Actor,
        timestamp: String,
    ) -> Self {
        Self {
            change_id,
            parents,
            predecessors: Vec::new(),
            tree,
            description,
            author,
            timestamp,
            provenance: Provenance::default(),
            conflicts: Vec::new(),
            authored_at: None,
            committer: None,
            signature: None,
            tag: None,
        }
    }
}

/// A tag: a fixed name for a commit. With a message it is what Git calls an
/// annotated tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub target: ObjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tagger: Option<Actor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// A workspace: one working directory with its own working-copy change.
/// Each agent gets one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// The working-copy commit (`@` inside this workspace).
    pub wc: ObjectId,
    /// Absolute path of the working directory.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Actor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    pub created_at: String,
    /// The bookmark this workspace is on: committing moves it forward, like
    /// a Git branch that is checked out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bookmark: Option<String>,
}

/// The state of the repository after an operation: which commits are
/// visible and where every workspace and bookmark points.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub workspaces: BTreeMap<String, WorkspaceInfo>,
    pub bookmarks: BTreeMap<String, ObjectId>,
    /// Commits kept visible even if no workspace or bookmark points at them.
    pub heads: BTreeSet<ObjectId>,
    /// Last known bookmarks of each remote (`main@origin`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub remotes: BTreeMap<String, BTreeMap<String, ObjectId>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, Tag>,
    /// Other targets of bookmarks that two concurrent operations moved to
    /// different places, until someone sets the bookmark again.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bookmark_conflicts: BTreeMap<String, BTreeSet<ObjectId>>,
    /// Stashed changes, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stashes: Vec<ObjectId>,
}

impl View {
    /// Every commit this view keeps visible directly.
    pub fn tips(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.workspaces
            .values()
            .map(|w| w.wc)
            .chain(self.bookmarks.values().copied())
            .chain(self.heads.iter().copied())
            .chain(self.remotes.values().flat_map(|b| b.values().copied()))
            .chain(self.tags.values().map(|t| t.target))
            .chain(self.bookmark_conflicts.values().flatten().copied())
            .chain(self.stashes.iter().copied())
    }
}
