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

pub mod assets;
pub mod atomic;
pub mod conflict;
pub mod error;
pub mod frontmatter;
pub mod image;
pub mod index;
pub mod markdown;
pub mod paths;
pub mod presence;
pub mod project;
pub mod recent;
pub mod scan;
pub mod watcher;

pub use assets::ImportedAsset;
pub use conflict::{Conflict, Keep, Resolved};
pub use error::{Error, Result};
pub use index::{CorruptFile, Index};
pub use presence::{Peer, Presence, PresenceHandle};
pub use project::{Enhanced, Project, ProjectMeta, ProjectSummary, SaveOutcome};
pub use recent::RecentProject;
pub use scan::{Cancel, ScanProgress};
pub use watcher::{Strategy, Watcher};
