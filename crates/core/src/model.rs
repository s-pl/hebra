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
}

/// A snapshot of files, keyed by `/`-separated path relative to the
/// workspace root. Flat on purpose for the MVP; it will become a Merkle
/// tree of directories once large repositories matter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    pub files: BTreeMap<String, FileEntry>,
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
}

impl Actor {
    #[must_use]
    pub fn human(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: ActorKind::Human,
            on_behalf_of: None,
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
    /// Both sides changed a binary file; ours was kept.
    Binary,
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
}

/// The state of the repository after an operation: which commits are
/// visible and where every workspace and bookmark points.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub workspaces: BTreeMap<String, WorkspaceInfo>,
    pub bookmarks: BTreeMap<String, ObjectId>,
    /// Commits kept visible even if no workspace or bookmark points at them.
    pub heads: BTreeSet<ObjectId>,
}
