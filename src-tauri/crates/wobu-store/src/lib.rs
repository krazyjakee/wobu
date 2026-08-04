//! Persistence for Wobu.
//!
//! A project is a self-contained folder of Markdown, not a database file
//! (`docs/02-data-model.md`). This crate owns that folder — reading it, writing
//! it safely, and maintaining the SQLite index that makes it queryable.
//!
//! The split worth remembering:
//!
//! - **The folder is canonical.** World entities live in Markdown and immutable
//!   generation history lives in JSON inside the project directory, so it can
//!   sit on a NAS and be opened by anyone who can see the path.
//! - **The index is disposable.** It lives in local app data keyed by project
//!   ULID, never inside the folder, because SQLite's locking is unsafe over
//!   SMB/NFS. Deleting it is always safe; it rebuilds from the Markdown.

pub mod apply;
pub mod assets;
pub mod atomic;
pub mod conflict;
pub mod error;
pub mod frontmatter;
pub mod generations;
pub mod image;
pub mod index;
pub mod lora;
pub mod markdown;
pub mod paths;
pub mod peer;
pub mod presence;
pub mod project;
pub mod recent;
pub mod scan;
pub mod thumbs;
pub mod transfer;
pub mod watcher;
pub mod wiki;

pub use apply::{Applied, ApplyReport, Decision, Incoming, Outgoing, Plan, Refused};
pub use assets::{ImportWarning, ImportedAsset, StoredMesh};
pub use conflict::{Conflict, Keep, Resolved};
pub use error::{Error, Result};
pub use index::{CorruptFile, GenerationPage, GenerationPageRequest, GenerationSummary, Index};
pub use presence::{Peer, Presence, PresenceHandle};
pub use project::{
    AssetUsage, AssetUsageRole, DEFAULT_SPEND_CEILING_USD_MICROS, Enhanced, Project, ProjectMeta,
    ProjectSummary, ReconcileObservation, ReconcilePlan, SaveOutcome,
};
pub use recent::RecentProject;
pub use scan::{Cancel, ScanProgress};
pub use thumbs::{ThumbTarget, Thumbnail};
pub use transfer::{TransferBundle, TransferCandidate, TransferOutcome, TransferPreview};
pub use watcher::{Change as WatchChange, Strategy, Watcher};
pub use wiki::{WikiExport, WikiSnapshot};
