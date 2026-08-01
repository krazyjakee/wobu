//! Domain types for Wobu.
//!
//! This crate is pure: no IO, no async, no Tauri. Everything the rest of the
//! workspace agrees on lives here — the [`Node`] record, the [`Link`] influence
//! edge, assets, generations, and the kind registry that drives both the UI and
//! the on-disk layout.
//!
//! See `docs/02-data-model.md`.

pub mod asset;
pub mod error;
pub mod generation;
pub mod kind;
pub mod link;
pub mod node;
pub mod peer;
pub mod preset;
pub mod schema;
pub mod slug;

pub use asset::{Asset, AssetKind, AssetLink, AssetRef, AssetRole};
pub use error::{Error, Result};
pub use generation::{
    FragmentTarget, Generation, InfluenceSnapshot, SnapshotFragment, SnapshotLayer,
};
pub use kind::{KindDef, Layer, NodeKind, SectionDef, SectionValueKind, kind_def, kind_registry};
pub use link::{Link, LinkEdge, LinkRole};
pub use node::{
    Description, DescriptionState, EnhanceStamp, Node, NodeSummary, SectionValue, SourceStamp,
    validate_parent,
};
pub use preset::{
    ANY_KIND, ImageConstraints, Preset, PresetGeneration, PresetView, SectionPriority,
    TURNAROUND_IMAGE_CONSTRAINTS, default_preset, preset, preset_registry, presets_for,
};
pub use schema::{HEX_COLOR_PATTERN, PALETTE_KEY, description_schema, is_hex_color};
pub use slug::{is_valid_slug, slugify, unique_slug};

/// Every id in Wobu is a ULID: lexicographically sortable, generated offline,
/// and safe to use as a filename.
pub type Id = ulid::Ulid;

/// Mint a new id. Wrapped so the rest of the workspace never depends on which
/// constructor the `ulid` crate happens to expose.
pub fn new_id() -> Id {
    ulid::Ulid::generate()
}

/// Bumped whenever the on-disk format changes in a way that requires a
/// migration. Written into `project.json` and checked on open.
pub const SCHEMA_VERSION: u32 = 1;
