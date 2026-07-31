use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{0} is not a Wobu project (no project.json)")]
    NotAProject(PathBuf),

    #[error("{0} already exists")]
    AlreadyExists(PathBuf),

    #[error(
        "this project was written by a newer version of Wobu (schema {found}, this build understands {supported})"
    )]
    SchemaTooNew { found: u32, supported: u32 },

    #[error("no node with id {0}")]
    NoSuchNode(String),

    #[error("malformed node file {path}: {reason}")]
    Malformed { path: PathBuf, reason: String },

    #[error("{0} is missing its YAML frontmatter")]
    MissingFrontmatter(PathBuf),

    #[error("the project folder is read-only")]
    ReadOnly,

    /// The folder was reachable when the project was opened and is not now.
    ///
    /// Distinct from [`Error::NotAProject`], which is a folder the user picked
    /// that never was one. This is a share that went away underneath an open
    /// session, and the difference matters: the index is still a complete copy
    /// of the world, so the right response is to hold on and wait rather than
    /// to close the project.
    #[error("the project folder is not reachable — the share may be unmounted")]
    Disconnected,

    #[error("no project is open")]
    NoProjectOpen,

    #[error("yaml error: {0}")]
    Yaml(#[from] serde_norway::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("index error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Core(#[from] wobu_core::Error),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Error {
        Error::Io { path: path.into(), source }
    }
}
