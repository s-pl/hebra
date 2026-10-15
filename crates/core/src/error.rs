use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not inside a Hebra workspace (run `hb init` first)")]
    NotAWorkspace,
    #[error("a Hebra repository already exists at {0}")]
    AlreadyInitialized(PathBuf),
    #[error("revision `{0}` not found")]
    RevisionNotFound(String),
    #[error("revision `{0}` is ambiguous; use a longer prefix")]
    AmbiguousRevision(String),
    #[error("workspace `{0}` already exists")]
    WorkspaceExists(String),
    #[error("workspace `{0}` not found")]
    WorkspaceNotFound(String),
    #[error("invalid id `{0}`")]
    InvalidId(String),
    #[error("invalid path `{0}`")]
    InvalidPath(String),
    #[error("object {0} is corrupt (hash mismatch)")]
    CorruptObject(String),
    #[error("{0}")]
    InvalidArgument(String),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("file walk error: {0}")]
    Walk(#[from] ignore::Error),
}

impl Error {
    /// Stable machine-readable code, used in `--json` error output.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAWorkspace => "not_a_workspace",
            Self::AlreadyInitialized(_) => "already_initialized",
            Self::RevisionNotFound(_) => "revision_not_found",
            Self::AmbiguousRevision(_) => "ambiguous_revision",
            Self::WorkspaceExists(_) => "workspace_exists",
            Self::WorkspaceNotFound(_) => "workspace_not_found",
            Self::InvalidId(_) => "invalid_id",
            Self::InvalidPath(_) => "invalid_path",
            Self::CorruptObject(_) => "corrupt_object",
            Self::InvalidArgument(_) => "invalid_argument",
            Self::Io { .. } => "io",
            Self::Db(_) => "database",
            Self::Json(_) => "serialization",
            Self::Walk(_) => "walk",
        }
    }
}

/// Attaches a path to I/O errors.
pub(crate) trait IoContext<T> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T> {
        self.map_err(|source| Error::Io {
            path: path.into(),
            source,
        })
    }
}
