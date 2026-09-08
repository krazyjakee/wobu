//! Where the boxes are: Flow canvas layout, stored beside narrative source and
//! never inside it (#185).
//!
//! ## The invariant this module exists to keep
//!
//! **Moving a box must not change the story.** Not the source bytes, not a
//! revision, not freshness, not an approval, not a fingerprint, not an export.
//! `wobu-narrative` enforces half of that by construction — there is no `x`,
//! no `y` and no `collapsed` anywhere in the source model, and every struct in
//! it refuses unknown fields, so a coordinate cannot be smuggled into a scene
//! document even by hand. This module is the other half: a separate file, in a
//! separate directory, keyed by the ids the source already mints, holding
//! presentation notes and geometry, never narrative dialogue.
//!
//! [`crate::narrative::source_fingerprint`] is the checkable form of the
//! claim. It cannot reach `narrative/layout/`, so a layout write cannot move
//! it, and the tests hash it either side of a drag.
//!
//! ## We merge this file, and we merge nothing else
//!
//! `docs/07-file-shares.md` says Wobu never merges: the loser of a race lands
//! beside the winner as a `.conflict-` sibling and a human resolves it. That
//! rule is right for prose, and it is wrong here, deliberately:
//!
//! - Two writers dragging two different boxes is not a semantic disagreement.
//!   There is no version of the arrangement that is "theirs" as opposed to
//!   "ours"; there is one canvas with two boxes on it, and both moves are
//!   correct.
//! - A diff card over a coordinate is a diff card nobody will read. The
//!   conflict card works because it is rare and always about words somebody
//!   wrote. Raising one every time two people had a scene open would train
//!   people to dismiss the card without looking — and the next one would be a
//!   paragraph.
//! - A conflicted layout must never be able to block a source save, and the
//!   cleanest way to guarantee that is for layout to live in a different file
//!   that resolves itself. Saving a scene does not touch this file at all.
//!
//! So: **last write wins, per node id**, over the union of both sides.
//! Somebody may lose the drag they made in the same second as a collaborator;
//! nobody loses their arrangement.
//!
//! ### What the merge cannot do, stated plainly
//!
//! - **It resolves by wall clock.** Each entry carries the `updatedAt` of
//!   whoever set it, and two machines with skewed clocks resolve in favour of
//!   the fast one, not the recent one. There is no vector clock and no
//!   causality here, because acquiring one would mean a per-peer table for a
//!   file whose worst-case loss is a rectangle in the wrong place.
//! - **Exact ties break by content hash**, not by merit. That is chosen only
//!   because it is *deterministic*: two machines merging the same pair of
//!   files in opposite directions have to reach the same answer, or the two
//!   folders never converge and the file ping-pongs forever.
//! - **Node positions merge by union.** [`reconcile`] drops deleted source IDs.
//!   Groups and notes carry version-2 deletion timestamps; deletion wins ties
//!   and older copies cannot resurrect them. A later edit can recreate them.
//! - **Groups and annotations merge whole.** A group's membership is one
//!   field of one entry, so two people adding different beats to the same
//!   group keeps only the later edit's membership. Per-member merging would
//!   need per-member timestamps, which is a lot of machinery to make a
//!   rectangle marginally righter.
//! - **The write is not a compare-and-swap.** Neither POSIX nor SMB has a
//!   rename that fails when the target moved. [`crate::atomic::merging_write`]
//!   re-reads bytes and re-merges up to three times, then defers a busy save.
//!   The remaining check-to-rename window can still lose a simultaneous drag.
//!
//! ## Nothing here may fail to open a scene
//!
//! [`load`] returns no `Result`. A missing file, a truncated file, a file
//! written by a newer Wobu, a file describing a different graph and a file
//! full of ids that no longer exist all produce the same thing: the best
//! layout available, plus [`LayoutNotice`]s. Every one of those is
//! non-blocking by construction, because a cosmetic sidecar that can stop a
//! writer opening their scene is worse than no sidecar at all.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wobu_narrative::{BeatId, ChoiceId, EntityId, OutcomeId, Scene, SceneId};

mod io;
pub use io::{MAX_LAYOUT_BYTES, graph_at, manifest, observation, read_document, read_raw};

use crate::atomic::{self, Stamp};
use crate::error::{Error, Result};

/// The layout format this build reads and writes.
///
/// Its own version, separate from `wobu_core::SCHEMA_VERSION` and from
/// `wobu_narrative::SOURCE_SCHEMA_VERSION`, and that separation is load
/// bearing: bumping the layout format must never look like a source migration,
/// or moving a box would make every project claim it needed one.
pub const LAYOUT_SCHEMA_VERSION: u32 = 3;

/// The presentation tree. Everything that must not see layout — a compiler
/// input set, a build fingerprint, an export, a `git add` of source — excludes
/// exactly this one prefix.
pub const LAYOUT_DIR: &str = "narrative/layout";
const SCENE_LAYOUT_DIR: &str = "narrative/layout/scenes";
const ARC_LAYOUT_DIR: &str = "narrative/layout/arcs";
/// The infix a corrupt sidecar is parked under. Deliberately *not*
/// `crate::conflict::MARKER`: a conflict sibling is somebody's only copy of a
/// paragraph and only a human may delete one, while this is a broken cosmetic
/// file, and the conflict card must never be handed one to arbitrate.
pub const CORRUPT_MARKER: &str = "corrupt";

/* ── what a layout file describes ─────────────────────────────────────────── */

/// Which graph a layout file is for.
///
/// Scene graphs use stable SceneId, quest graphs use the World quest EntityId.
/// Legacy slug-keyed Arc files remain readable without an opening migration;
/// the all-scenes view uses that form. Names never replace stable quest IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GraphKey {
    Scene { scene: SceneId },
    Arc { arc: String },
    Quest { quest: EntityId },
}

impl GraphKey {
    pub fn of_scene(scene: SceneId) -> GraphKey {
        GraphKey::Scene { scene }
    }

    /// The project-relative path this graph's layout lives at.
    ///
    /// An arc slug is re-slugified rather than trusted. The caller is inside
    /// the app, but this string becomes a filename on an SMB share, and a
    /// stray `..` or `:` in it would be either a refused write or a write
    /// somewhere it should not be — the exact class of thing
    /// `ProjectRelativePath` exists to stop, applied one layer earlier where a
    /// helpful name can still be recovered.
    pub fn rel(&self) -> Result<String> {
        match self {
            GraphKey::Scene { scene } => Ok(format!("{SCENE_LAYOUT_DIR}/{scene}.json")),
            GraphKey::Quest { quest } => Ok(format!("narrative/layout/quests/{quest}.json")),
            GraphKey::Arc { arc } => {
                let slug = wobu_core::slugify(arc)?;
                Ok(format!("{ARC_LAYOUT_DIR}/{slug}.json"))
            }
        }
    }
}

/// One thing on the canvas that has a position.
///
/// A tagged union of the source's own identity types rather than a bare ULID,
/// so a beat id cannot be read back as a scene id by a file that was
/// hand-edited or written by a build that disagreed. The tag costs six bytes
/// per entry and buys the same guarantee `wobu-narrative`'s distinct id types
/// buy in memory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum NodeKey {
    Scene(SceneId),
    Beat(BeatId),
    Choice(ChoiceId),
    Outcome(OutcomeId),
    QuestStage { quest: EntityId, stage: String },
}

impl fmt::Display for NodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeKey::Scene(id) => write!(f, "scene:{id}"),
            NodeKey::Beat(id) => write!(f, "beat:{id}"),
            NodeKey::Choice(id) => write!(f, "choice:{id}"),
            NodeKey::Outcome(id) => write!(f, "outcome:{id}"),
            NodeKey::QuestStage { quest, stage } => write!(f, "stage:{quest}:{stage}"),
        }
    }
}

impl FromStr for NodeKey {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<NodeKey, String> {
        let (noun, id) =
            value.split_once(':').ok_or_else(|| format!("{value} is not a node key"))?;
        let bad = || format!("{value} is not a node key");
        match noun {
            "scene" => SceneId::from_str(id).map(NodeKey::Scene).map_err(|_| bad()),
            "beat" => BeatId::from_str(id).map(NodeKey::Beat).map_err(|_| bad()),
            "choice" => ChoiceId::from_str(id).map(NodeKey::Choice).map_err(|_| bad()),
            "outcome" => OutcomeId::from_str(id).map(NodeKey::Outcome).map_err(|_| bad()),
            "stage" => {
                let (quest, stage) = id.split_once(':').ok_or_else(bad)?;
                if stage.is_empty() || stage.len() > 1024 {
                    return Err(bad());
                }
                Ok(NodeKey::QuestStage {
                    quest: quest.parse().map_err(|_| bad())?,
                    stage: stage.into(),
                })
            }
            _ => Err(bad()),
        }
    }
}

impl From<NodeKey> for String {
    fn from(key: NodeKey) -> String {
        key.to_string()
    }
}

impl TryFrom<String> for NodeKey {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<NodeKey, String> {
        NodeKey::from_str(&value)
    }
}

macro_rules! presentation_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(wobu_core::Id);

        impl $name {
            pub fn new() -> $name {
                $name(wobu_core::new_id())
            }
        }

        impl Default for $name {
            fn default() -> $name {
                $name::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

presentation_id!(
    /// A collapsible group of canvas nodes.
    ///
    /// Its own identity, minted here, and deliberately *not* one of
    /// `wobu-narrative`'s: a group is a thing a person drew a box around, it
    /// has no meaning to the compiler, and giving it a narrative id would be
    /// the first step towards it acquiring narrative meaning.
    GroupId
);
presentation_id!(
    /// A pinned note on the canvas.
    AnnotationId
);

/// Whether the canvas is arranging itself or obeying the user.
///
/// Stored per graph, because it is a statement about this canvas and not a
/// preference: a colleague opening the scene has to see the arrangement the
/// author made, and an automatic canvas that silently became manual on their
/// machine would freeze whatever the auto-layout last produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutMode {
    /// Positions are computed. Saved positions are still kept, so switching
    /// back to manual restores the arrangement rather than starting over.
    #[default]
    Automatic,
    Manual,
}

/// Everything remembered about one node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeLayout {
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupId>,
    /// When this entry was last set, by the clock of the machine that set it.
    /// The whole of the merge rule; see the module docs for what that costs.
    pub updated_at: DateTime<Utc>,
}

impl NodeLayout {
    pub fn at(x: f64, y: f64) -> NodeLayout {
        NodeLayout { x, y, collapsed: false, group: None, updated_at: Utc::now() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Group {
    pub id: GroupId,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<NodeKey>,
    pub updated_at: DateTime<Utc>,
}

/// A pinned note.
///
/// Prose, and therefore the one thing in this file a person actually wrote —
/// which is worth being explicit about, because it is also the one thing in
/// here that a last-write-wins merge can lose words from. It is accepted for
/// the same reason a sticky note on a whiteboard is not version controlled:
/// an annotation is a margin comment on an arrangement, it is never compiled,
/// exported or translated, and #151 keeps authored narrative text in the
/// source document where the never-merge rule still applies to it in full.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Annotation {
    pub id: AnnotationId,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Which node this note is pinned to, if any. Keyed by id like everything
    /// else, so a note stays on its beat through a rename and is collected
    /// with it on a delete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached_to: Option<NodeKey>,
    pub updated_at: DateTime<Utc>,
}

/// One layout file.
///
/// `BTreeMap` rather than `HashMap` throughout, so the serialised bytes are a
/// function of the contents and not of a hash seed. Two machines that agree on
/// the arrangement must produce the same file, or every open would look like a
/// change to the watcher and to every sync client the folder sits under.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Layout {
    pub schema_version: u32,
    pub graph: GraphKey,
    #[serde(default)]
    pub mode: LayoutMode,
    /// Stamped separately from the nodes because the mode is merged
    /// separately: switching a canvas to manual and dragging one box in it are
    /// two edits, and a shared timestamp would make the second undo the first.
    pub mode_updated_at: DateTime<Utc>,
    #[serde(default)]
    pub nodes: BTreeMap<NodeKey, NodeLayout>,
    #[serde(default)]
    pub groups: BTreeMap<GroupId, Group>,
    #[serde(default)]
    pub annotations: BTreeMap<AnnotationId, Annotation>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub removed_groups: BTreeMap<GroupId, DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub removed_annotations: BTreeMap<AnnotationId, DateTime<Utc>>,
}

impl Layout {
    pub fn empty(graph: GraphKey) -> Layout {
        Layout {
            schema_version: LAYOUT_SCHEMA_VERSION,
            graph,
            mode: LayoutMode::default(),
            // The epoch rather than now, so an empty layout that was never
            // written always loses a merge against one that was. Stamping it
            // with the current time would let simply opening a scene overwrite
            // a collaborator's choice of manual mode.
            mode_updated_at: DateTime::UNIX_EPOCH,
            nodes: BTreeMap::new(),
            groups: BTreeMap::new(),
            annotations: BTreeMap::new(),
            removed_groups: BTreeMap::new(),
            removed_annotations: BTreeMap::new(),
        }
    }

    /// Put a node somewhere, stamping the edit.
    pub fn place(&mut self, key: NodeKey, x: f64, y: f64) {
        let entry = self.nodes.entry(key).or_insert_with(|| NodeLayout::at(x, y));
        entry.x = x;
        entry.y = y;
        entry.updated_at = Utc::now();
    }

    pub fn set_collapsed(&mut self, key: NodeKey, collapsed: bool) {
        let entry = self.nodes.entry(key).or_insert_with(|| NodeLayout::at(0.0, 0.0));
        entry.collapsed = collapsed;
        entry.updated_at = Utc::now();
    }

    pub fn set_mode(&mut self, mode: LayoutMode) {
        self.mode = mode;
        self.mode_updated_at = Utc::now();
    }

    pub fn upsert_group(&mut self, mut group: Group) {
        group.updated_at = Utc::now();
        self.groups.insert(group.id, group);
    }

    pub fn upsert_annotation(&mut self, mut annotation: Annotation) {
        annotation.updated_at = Utc::now();
        self.annotations.insert(annotation.id, annotation);
    }

    /// Which of `wanted` this layout has no position for.
    ///
    /// The canvas lays these out automatically. A new beat, a beat somebody
    /// else added, and every beat in a scene that has never been arranged all
    /// arrive here, which is why it is a list and not an error.
    pub fn unplaced(&self, wanted: &BTreeSet<NodeKey>) -> Vec<NodeKey> {
        wanted.iter().cloned().filter(|key| !self.nodes.contains_key(key)).collect()
    }

    fn to_json(&self) -> Result<String> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }
}

/// Every node key one scene puts on a canvas.
///
/// Beats, and the choices and outcomes leaving them. Dialogue slots and
/// variants are deliberately absent: they are rows inside a beat's node, not
/// nodes, and giving them positions would mean the canvas had opinions about
/// the inside of a box.
pub fn scene_keys(scene: &Scene) -> BTreeSet<NodeKey> {
    let mut keys = BTreeSet::new();
    keys.insert(NodeKey::Scene(scene.id));
    for beat in &scene.beats {
        keys.insert(NodeKey::Beat(beat.id));
        keys.extend(beat.choices.iter().map(|choice| NodeKey::Choice(choice.id)));
        keys.extend(beat.outcomes.iter().map(|outcome| NodeKey::Outcome(outcome.id)));
    }
    keys
}

/* ── loading, which never fails ───────────────────────────────────────────── */

/// Something the canvas should know but must not be stopped by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutNotice {
    /// No sidecar. Ordinary for a scene nobody has arranged yet.
    Missing { rel: String },
    /// There was a file and it could not be understood. Its bytes are kept:
    /// `parked` names where they went, once something tries to save over them.
    Unreadable { rel: String, reason: String },
    /// Written by a newer Wobu. Read as absent and, crucially, never written
    /// over — downgrading a colleague's file would destroy an arrangement to
    /// protect a rectangle.
    NewerSchema { rel: String, found: u32, supported: u32 },
    /// The file at this path describes a different graph — a copied project
    /// folder, or a hand-moved file. Ignored rather than adopted.
    WrongGraph { rel: String },
    /// Source ids with no saved position. These fall back to automatic layout.
    Unplaced { nodes: Vec<NodeKey> },
    /// Saved positions for ids the source no longer contains. Dropped.
    Stale { nodes: Vec<NodeKey> },
}

impl LayoutNotice {
    /// Every notice is non-blocking. The method exists so a caller can say so
    /// out loud rather than assuming it, and so that adding a blocking variant
    /// later has to be a deliberate edit here.
    pub fn is_blocking(&self) -> bool {
        false
    }
}

/// A layout and everything that was wrong with getting it.
#[derive(Debug, Clone)]
pub struct LayoutLoad {
    pub layout: Layout,
    pub notices: Vec<LayoutNotice>,
    /// What we last saw on disk, or `None` if there was nothing readable.
    /// Carried so a caller can tell a sidecar that has changed under it from
    /// one that has not; the write path re-reads regardless.
    pub stamp: Option<Stamp>,
}

/// Read a layout file, degrading to an empty layout for every possible reason
/// it could not be read.
///
/// Returns no `Result` on purpose. There is no failure mode here that should
/// reach a user as an error, because there is no failure mode here that costs
/// them anything but an arrangement.
pub fn load(root: &Path, graph: &GraphKey) -> LayoutLoad {
    let Ok(rel) = graph.rel() else {
        // An arc name that will not slugify. Nothing to read, nothing to
        // report to the person: the canvas simply lays itself out.
        return LayoutLoad {
            layout: Layout::empty(graph.clone()),
            notices: Vec::new(),
            stamp: None,
        };
    };
    let empty = |notice: Option<LayoutNotice>| LayoutLoad {
        layout: Layout::empty(graph.clone()),
        notices: notice.into_iter().collect(),
        stamp: None,
    };

    let path = match path_of(root, graph) {
        Ok(path) => path,
        Err(error) => {
            return empty(Some(LayoutNotice::Unreadable { rel, reason: error.to_string() }));
        }
    };
    let read = match io::read_bounded(&path) {
        Ok(Some(read)) => read,
        Ok(None) => return empty(Some(LayoutNotice::Missing { rel })),
        // An unreadable share, a permission problem, a file that vanished
        // mid-read. Reported as unreadable rather than propagated: the scene
        // still opens.
        Err(error) => {
            return empty(Some(LayoutNotice::Unreadable { rel, reason: error.to_string() }));
        }
    };
    let (text, stamp) = read;

    match probe_version(&text) {
        Some(version) if version > LAYOUT_SCHEMA_VERSION => {
            return empty(Some(LayoutNotice::NewerSchema {
                rel,
                found: version,
                supported: LAYOUT_SCHEMA_VERSION,
            }));
        }
        _ => {}
    }

    let mut layout: Layout = match serde_json::from_str(&text) {
        Ok(layout) => layout,
        Err(error) => {
            return empty(Some(LayoutNotice::Unreadable { rel, reason: error.to_string() }));
        }
    };
    if let Err(error) = layout.validate() {
        return empty(Some(LayoutNotice::Unreadable { rel, reason: error.to_string() }));
    }
    if layout.graph != *graph {
        return empty(Some(LayoutNotice::WrongGraph { rel }));
    }
    layout.prune_removed();
    LayoutLoad { layout, notices: Vec::new(), stamp: Some(stamp) }
}

/// Read `schemaVersion` and nothing else, permissively.
///
/// Same trick as `wobu_narrative::source`: a file from a newer build has to be
/// reported as a newer file, not as a pile of fields this build cannot parse,
/// or the obvious response is to delete the fields.
fn probe_version(text: &str) -> Option<u32> {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(rename = "schemaVersion")]
        schema_version: Option<u32>,
    }
    serde_json::from_str::<Probe>(text).ok().and_then(|probe| probe.schema_version)
}

/// Drop saved positions for ids the source no longer has, and report the ids
/// the source has that the layout does not.
///
/// Both halves are non-destructive on disk: this edits the loaded layout, and
/// the file only changes when something saves. That ordering is deliberate —
/// a read-only share must open, and a project opened to be looked at must not
/// be rewritten by the looking.
pub fn reconcile(loaded: &mut LayoutLoad, present: &BTreeSet<NodeKey>) {
    let stale: Vec<NodeKey> = loaded
        .layout
        .nodes
        .keys()
        .cloned()
        .chain(loaded.layout.groups.values().flat_map(|group| group.members.iter().cloned()))
        .chain(loaded.layout.annotations.values().filter_map(|note| note.attached_to.clone()))
        .filter(|key| !present.contains(key))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !stale.is_empty() {
        for key in &stale {
            loaded.layout.nodes.remove(key);
        }
        for group in loaded.layout.groups.values_mut() {
            group.members.retain(|member| present.contains(member));
        }
        loaded
            .layout
            .annotations
            .values_mut()
            .filter(|annotation| {
                annotation.attached_to.as_ref().is_some_and(|key| !present.contains(key))
            })
            // An annotation pinned to a deleted beat keeps its words and loses
            // its anchor. Deleting the note would throw away the one thing in
            // this file a person typed.
            .for_each(|annotation| annotation.attached_to = None);
        loaded.notices.push(LayoutNotice::Stale { nodes: stale });
    }

    let unplaced = loaded.layout.unplaced(present);
    if !unplaced.is_empty() {
        loaded.notices.push(LayoutNotice::Unplaced { nodes: unplaced });
    }
}

/// [`load`] plus [`reconcile`] for one scene: the call the canvas actually
/// makes.
pub fn load_for_scene(root: &Path, scene: &Scene) -> LayoutLoad {
    let mut loaded = load(root, &GraphKey::of_scene(scene.id));
    reconcile(&mut loaded, &scene_keys(scene));
    loaded
}

/* ── saving, which merges ─────────────────────────────────────────────────── */

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutSave {
    Written(Stamp),
    /// Not written, and nothing lost. The only reason: the file on disk was
    /// written by a newer Wobu, and this build has no way to merge into a
    /// shape it does not know. Overwriting would trade somebody's whole
    /// arrangement for one drag.
    Deferred {
        rel: String,
        found: u32,
        supported: u32,
    },
}

/// Merge `layout` into whatever is on disk and land it.
///
/// `prune_to`, when supplied, is the set of node keys the source currently
/// contains, and entries outside it are dropped as part of the same write.
/// Handing it in rather than deriving it here is what keeps this module from
/// needing to know how to read a scene — and what makes the pruning honest:
/// the caller passes the scene it is looking at, so an entry that scene does
/// not contain is either deleted or belongs to a version of the scene this
/// machine has not read.
///
/// The second case is the one worth stating: a beat a collaborator added,
/// whose scene file has not reached us yet, can have its saved position
/// pruned here. The cost is that beat opening at an automatic position with an
/// [`LayoutNotice::Unplaced`] beside it — which is exactly the graceful
/// degradation this module is specified to do — and never the loss of anything
/// a person wrote. Pass `None` to merge without pruning.
pub fn save(
    root: &Path,
    peer: &str,
    layout: &Layout,
    prune_to: Option<&BTreeSet<NodeKey>>,
) -> Result<LayoutSave> {
    layout.validate()?;
    let rel = layout.graph.rel()?;
    let path = path_of(root, &layout.graph)?;
    let stamp = atomic::merging_write(root, &path, MAX_LAYOUT_BYTES, |current| {
        // This runs for every fresh read, including retries after a concurrent
        // writer. A schema check outside this loop would permit downgrades.
        path_of(root, &layout.graph)?;
        let mut merged = layout.clone();
        if let Some(text) = current {
            if let Some(found) = probe_version(text)
                && found > LAYOUT_SCHEMA_VERSION
            {
                return Err(Error::SchemaTooNew { found, supported: LAYOUT_SCHEMA_VERSION });
            }
            match io::parse(text) {
                Ok(theirs) if theirs.graph == merged.graph => merge_into(&mut merged, theirs),
                _ => {
                    io::preserve_unreadable(root, &path, text, peer)?;
                }
            }
        }
        if let Some(present) = prune_to {
            let mut loaded = LayoutLoad { layout: merged, notices: Vec::new(), stamp: None };
            reconcile(&mut loaded, present);
            merged = loaded.layout;
        }
        merged.schema_version = LAYOUT_SCHEMA_VERSION;
        merged.validate()?;
        merged.to_json()
    });
    match stamp {
        Ok(stamp) => Ok(LayoutSave::Written(stamp)),
        Err(Error::SchemaTooNew { found, supported }) => {
            Ok(LayoutSave::Deferred { rel, found, supported })
        }
        Err(error) => Err(error),
    }
}

/// Union both sides, keeping the later edit of each entry.
fn merge_into(mine: &mut Layout, theirs: Layout) {
    if (theirs.mode_updated_at, digest(&theirs.mode)) > (mine.mode_updated_at, digest(&mine.mode)) {
        mine.mode = theirs.mode;
        mine.mode_updated_at = theirs.mode_updated_at;
    }
    merge_entries(&mut mine.nodes, theirs.nodes);
    merge_entries(&mut mine.groups, theirs.groups);
    merge_entries(&mut mine.annotations, theirs.annotations);
    merge_removals(&mut mine.removed_groups, theirs.removed_groups);
    merge_removals(&mut mine.removed_annotations, theirs.removed_annotations);
    mine.prune_removed();
}

fn merge_removals<K: Ord>(
    mine: &mut BTreeMap<K, DateTime<Utc>>,
    theirs: BTreeMap<K, DateTime<Utc>>,
) {
    for (key, stamp) in theirs {
        mine.entry(key).and_modify(|current| *current = (*current).max(stamp)).or_insert(stamp);
    }
}

/// The stamp a merge resolves on.
trait Stamped {
    fn updated_at(&self) -> DateTime<Utc>;
}

impl Stamped for NodeLayout {
    fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

impl Stamped for Group {
    fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

impl Stamped for Annotation {
    fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }
}

fn merge_entries<K, V>(mine: &mut BTreeMap<K, V>, theirs: BTreeMap<K, V>)
where
    K: Ord,
    V: Stamped + Serialize,
{
    for (key, theirs) in theirs {
        match mine.get(&key) {
            Some(ours) if !replaces(&theirs, ours) => {}
            _ => {
                mine.insert(key, theirs);
            }
        }
    }
}

/// Whether `candidate` beats `incumbent`, deterministically.
///
/// The clock decides, and an exact tie is broken by content hash. The
/// tie-break has no claim to being *right* — it exists so that two machines
/// merging the same two files in opposite directions reach the same answer.
/// Without it, a tie resolves in favour of whichever side happened to be
/// "theirs", the two folders never agree, and each sync round rewrites the
/// file and wakes every watcher on the share.
fn replaces<V: Stamped + Serialize>(candidate: &V, incumbent: &V) -> bool {
    match candidate.updated_at().cmp(&incumbent.updated_at()) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => digest(candidate) > digest(incumbent),
    }
}

fn digest<V: Serialize>(value: &V) -> [u8; 32] {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    *blake3::hash(&bytes).as_bytes()
}

/* ── deletion ─────────────────────────────────────────────────────────────── */

/// Remove one graph's layout. Missing is success.
pub fn delete(root: &Path, graph: &GraphKey) -> Result<()> {
    let path = path_of(root, graph)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::io(&path, error)),
    }
}

/// Delete scene layout files whose scene is gone, and report what went.
///
/// Deliberately **not** run at open. A half-mounted share, a sync client
/// part-way through a copy and a project being restored from backup all look
/// exactly like "most of the scenes have been deleted", and a sweep on that
/// evidence would erase every arrangement in the project in one pass. It is
/// called where the evidence is unambiguous — immediately after this machine
/// deleted a scene — and offered as a maintenance action otherwise.
pub fn sweep(root: &Path, live: &BTreeSet<SceneId>) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for (rel, path) in
        manifest(root)?.into_iter().filter(|(rel, _)| rel.starts_with("narrative/layout/scenes/"))
    {
        let Some(stem) = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()) else {
            continue;
        };
        // Anything whose name is not a scene id is left alone. A parked
        // corrupt file, or a name a human gave a file they were rescuing, is
        // not this function's business.
        let Ok(scene) = SceneId::from_str(&stem) else { continue };
        if live.contains(&scene) {
            continue;
        }
        std::fs::remove_file(&path).map_err(|error| Error::io(&path, error))?;
        removed.push(rel);
    }
    removed.sort();
    Ok(removed)
}

/// Whether a project-relative path is presentation metadata.
///
/// The one place anything else should ask. A caller that spelled the prefix
/// itself is a caller that can get it subtly wrong, and getting it wrong in
/// the permissive direction puts a coordinate in a shipped package.
pub fn is_layout_path(rel: &str) -> bool {
    rel == LAYOUT_DIR || rel.starts_with(&format!("{LAYOUT_DIR}/"))
}

/// Where a layout file for this graph would live. For the watcher and for
/// tests; nothing else needs to build the path itself.
pub fn path_of(root: &Path, graph: &GraphKey) -> Result<PathBuf> {
    io::safe_path(root, &graph.rel()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_key_round_trips_through_its_string_form() {
        let key = NodeKey::Beat(BeatId::new());
        assert_eq!(NodeKey::from_str(&key.to_string()).unwrap(), key);
    }

    #[test]
    fn a_node_key_will_not_read_a_beat_as_a_scene() {
        // The tag is the guarantee: the same ULID under two nouns is two
        // different keys, so a hand-edited file cannot move a scene by naming
        // a beat.
        let id = wobu_core::new_id();
        let beat = NodeKey::Beat(BeatId::from_raw(id));
        let scene = NodeKey::Scene(SceneId::from_raw(id));
        assert_ne!(beat, scene);
        assert_ne!(NodeKey::from_str(&beat.to_string()).unwrap(), scene);
    }

    #[test]
    fn layout_paths_are_under_the_presentation_tree() {
        let graph = GraphKey::of_scene(SceneId::new());
        assert!(is_layout_path(&graph.rel().unwrap()));
        assert!(!is_layout_path("narrative/scenes/kiln.yaml"));
        // A path that merely starts with the same letters is not inside it.
        assert!(!is_layout_path("narrative/layouts/kiln.json"));
    }

    #[test]
    fn an_arc_name_is_slugified_into_its_filename() {
        let graph = GraphKey::Arc { arc: "The Kiln Job".into() };
        assert_eq!(graph.rel().unwrap(), "narrative/layout/arcs/the-kiln-job.json");
    }

    #[test]
    fn a_tie_breaks_the_same_way_from_either_side() {
        let at = Utc::now();
        let a = NodeLayout { x: 1.0, y: 2.0, collapsed: false, group: None, updated_at: at };
        let b = NodeLayout { x: 9.0, y: 9.0, collapsed: false, group: None, updated_at: at };
        assert_ne!(replaces(&a, &b), replaces(&b, &a));
    }

    #[test]
    fn an_empty_layout_never_wins_the_mode() {
        // Opening a scene must not overwrite a collaborator's choice of manual
        // mode with the default this build made up on the way in.
        let graph = GraphKey::of_scene(SceneId::new());
        let mut theirs = Layout::empty(graph.clone());
        theirs.set_mode(LayoutMode::Manual);
        let mut ours = Layout::empty(graph);
        merge_into(&mut ours, theirs);
        assert_eq!(ours.mode, LayoutMode::Manual);
    }
}
