//! Persistence for Wobu.
//!
//! A project is a self-contained folder of Markdown, not a database file
//! (`docs/02-data-model.md`). This crate owns that folder — reading it, writing
//! it safely, and maintaining the SQLite index that makes it queryable.
//!
//! The split worth remembering:
//!
//! - **The folder is canonical.** Every fact about a world lives in Markdown
//!   inside the project directory, so it can sit on a NAS and be opened by
//!   anyone who can see the path.
//! - **The index is disposable.** It lives in local app data keyed by project
//!   ULID, never inside the folder, because SQLite's locking is unsafe over
//!   SMB/NFS. Deleting it is always safe; it rebuilds from the Markdown.

pub mod atomic;
pub mod error;
pub mod frontmatter;
pub mod index;
pub mod markdown;
pub mod paths;
pub mod project;
pub mod recent;
pub mod watcher;

pub use error::{Error, Result};
pub use index::Index;
pub use project::{Project, ProjectMeta, ProjectSummary, SaveOutcome};
pub use recent::RecentProject;
pub use watcher::{Strategy, Watcher};
