//! Narrative source, diagnostics and canvas layout, as coarse commands.
//!
//! Glue, like the rest of `commands/`: the source model is `wobu-narrative`'s
//! and the files are `wobu-store`'s. What is decided here is the *shape of the
//! conversation* with the webview, and three of those decisions are worth
//! stating because getting any of them wrong is expensive to undo later.
//!
//! ## Coarse, not chatty
//!
//! There is no `narrative_beat_add`, no `narrative_choice_set_label`, no
//! `narrative_outcome_set_destination`. A structural edit is expressed as
//! "here is the whole scene document now", because #156 requires that a canvas
//! edit and the equivalent form edit produce *identical source*, and the only
//! way to guarantee that is for both to go through one write. A per-field
//! command surface would mean two implementations of "duplicate a beat" — one
//! the canvas calls and one the form calls — and the first divergence between
//! them is a scene that reads differently depending on which tab created it.
//! A scene document is a few kilobytes; the bridge is not the constraint.
//!
//! ## The precondition travels on the document, not in a cache
//!
//! `SceneFile` carries the [`Stamp`] the file had when it was read, and
//! [`SceneFileView`] hands that straight to the webview, which hands it back on
//! save. There is deliberately no map of scene id → stamp anywhere in this
//! process. Scenes are not in the SQLite index yet (the remaining half of
//! #153), so such a map would be the *only* holder of the precondition that
//! stops two writers clobbering each other, and a cache that can go stale in
//! that role turns a detected conflict into a silent overwrite.
//!
//! [`Precondition`] is how a caller says which precondition it means, out
//! loud, rather than by the absence of an argument.
//!
//! ## Layout can never stop anybody
//!
//! Layout lives in its own files, behind its own commands, and
//! [`narrative_layout_save`] cannot fail: every reason a rectangle did not
//! reach disk comes back as an outcome to read, not an error to handle. That
//! is the command-layer half of #185's invariant. Saving a scene does not
//! touch a layout file and saving a layout does not touch a scene file, so
//! there is no arrangement anywhere that can block or fail a source save —
//! not because a caller remembered to order the two calls correctly, but
//! because they share nothing.
//!
//! ## Why `scene` crosses the bridge in the source file's own spelling
//!
//! Every envelope in this file is `camelCase`, as the rest of the command
//! surface is. The `scene` and `document` payloads inside them are not: they
//! are `wobu_narrative::Scene` and `StateDocument` serialised exactly as they
//! are written to YAML, which is `snake_case`. That is on purpose. A mirrored
//! camelCase DTO would be a second copy of the narrative model living in this
//! crate, and it would have to be updated in lockstep with a model whose whole
//! defence against silent data loss is `deny_unknown_fields`. The Source tab
//! (US-14) edits that YAML directly; making the Script tab speak a different
//! dialect of the same document would give the two views two vocabularies for
//! one file.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use wobu_narrative::{
    BeatId, ChoiceId, DestinationSite, Diagnostic, DialogueSlotId, OutcomeId, Problem, Scene,
    SceneCatalog, SceneId, Site, StateDocument, TextEntryId, VariantId,
};
use wobu_store::atomic::Stamp;
use wobu_store::{GraphKey, Layout, LayoutLoad, LayoutNotice, LayoutSave, Project, SourceSave};

use crate::error::{Code, CommandResult, WobuError};
use crate::state::AppState;

/* ── what the webview sees ────────────────────────────────────────────────── */

/// One scene as the Library lists it, without parsing its beats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneSummary {
    pub id: SceneId,
    pub name: String,
    /// The file name stem. Minted once at create and never recomputed, so it
    /// is stable across a rename — see `Project::create_scene`.
    pub slug: String,
    /// Project-relative, `/`-separated. Shown in the Source tab's header and
    /// used to match a conflict sibling to the scene it belongs to.
    pub rel: String,
}

/// A file in the scenes directory that could not be identified.
///
/// Listed rather than swallowed. A scene a sync client truncated has no
/// readable id, so it cannot appear as a scene, and a catalog that quietly
/// omitted it would present somebody's file as deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadableScene {
    pub rel: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneCatalogView {
    pub scenes: Vec<SceneSummary>,
    pub unreadable: Vec<UnreadableScene>,
}

/// One scene, whole, with the precondition for writing it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneFileView {
    /// The source document, in the source document's own spelling. See the
    /// module docs.
    pub scene: Scene,
    pub slug: String,
    pub rel: String,
    /// What the file looked like when this view was made. Opaque to the
    /// webview: it is carried and handed back, never inspected or constructed.
    /// `None` means "we believe there is no file", which is a claim a write can
    /// be checked against just as much as a stamp is.
    pub stamp: Option<Stamp>,
}

impl SceneFileView {
    pub(super) fn of(file: &wobu_store::SceneFile) -> SceneFileView {
        SceneFileView {
            scene: file.scene.clone(),
            slug: file.slug().to_string(),
            rel: file.rel.clone(),
            stamp: file.stamp.clone(),
        }
    }
}

/// The declared variables, and the precondition for writing them back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateFileView {
    pub document: StateDocument,
    /// `None` in a project that has never declared any state. A scene can
    /// exist long before its variables do, so that is an ordinary answer
    /// rather than a missing file to complain about.
    pub stamp: Option<Stamp>,
}

/// Which version of a file a write expects to be replacing.
///
/// Spelled out as three named cases rather than inferred from an
/// `Option<Stamp>`, because "I hold no stamp" and "I want whatever is there
/// now" are opposite intentions that an absent argument cannot tell apart —
/// and one of them silently overwrites.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Precondition {
    /// The file as this caller last read it. An edit that landed in between is
    /// a conflict: the incoming version is parked beside the winner and a
    /// person decides. This is what an open editor sends.
    Stamp { stamp: Stamp },
    /// The caller believes there is no file. Anything already there is a
    /// conflict rather than something to overwrite.
    New,
    /// Whatever is on disk at the moment of the write, read inside the same
    /// project lock the write takes.
    ///
    /// The undo path, and the exact analogue of `Project::save_node` reading
    /// the stamp out of the index: undo restores a version this session
    /// recorded, and refusing it because *this session's own* later save moved
    /// the file would make ⌘Z fail on every second press. It is worth being
    /// blunt about the cost — a collaborator's edit that arrived since is
    /// overwritten by an undo rather than parked, exactly as it is for nodes.
    Current,
}

impl Precondition {
    /// Resolve to the stamp `guarded_write` should compare against.
    pub(super) fn against(&self, on_disk: Option<Stamp>) -> Option<Stamp> {
        match self {
            Precondition::Stamp { stamp } => Some(stamp.clone()),
            Precondition::New => None,
            Precondition::Current => on_disk,
        }
    }
}

/* ── diagnostics ──────────────────────────────────────────────────────────── */

/// One problem, flattened onto the ids that identify the thing responsible.
///
/// `wobu_narrative::Site` is a nested enum, which is right in Rust and wrong
/// here: the canvas keys nodes by a beat/choice/outcome id and the Validation
/// list keys rows by the same, and making each of them destructure a
/// three-level union is how the two ends up reading the *nearly* same data.
/// Flat ids mean one obvious join, and #186's requirement that "the same beat,
/// choice, and outcome IDs appear in Flow, Script, Source, and diagnostics" is
/// then true by construction rather than by two careful implementations.
///
/// Everything here is derived — nothing is stored — so there is no version of
/// this that can disagree with the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticView {
    /// Which kind of element this is about, and therefore which id below is
    /// the one to select on.
    pub kind: &'static str,
    /// A stable machine-readable name for the problem. Distinct from
    /// `message`, which is prose that may be reworded; a filter or a badge
    /// switches on this.
    pub code: &'static str,
    /// The problem in the words `wobu-narrative` already wrote for it. Not
    /// restated here — a second wording is a second thing to keep true.
    pub message: String,
    /// Whether this is one of the destination problems, so the canvas can
    /// draw broken wiring without re-running the analysis or matching on
    /// codes it would have to be kept in step with.
    pub destination: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub beat_id: Option<BeatId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choice_id: Option<ChoiceId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_id: Option<OutcomeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_id: Option<DialogueSlotId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_id: Option<VariantId>,
    /// The participant or speaker a reference problem is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<wobu_core::Id>,
    /// Intents have no id — they are prose attached to a beat — so their
    /// position in the beat's list is the only thing that can address one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_index: Option<usize>,
    /// The entry of a supporting text asset a problem belongs to (#167). A
    /// separate field from `beat_id` because a caller turning a diagnostic into
    /// a selection has to open a different editor for each, and one id field
    /// holding either would make that a guess.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<TextEntryId>,
}

/// The stable name for a problem.
///
/// Written out rather than derived from the variant name, so renaming a Rust
/// variant cannot silently change a string the frontend filters on — the same
/// discipline `error::Code` keeps.
fn problem_code(problem: &Problem) -> &'static str {
    match problem {
        Problem::UnresolvedDestination => "unresolved_destination",
        Problem::UnknownClassification { .. } => "unknown_classification",
        Problem::DanglingBeat { .. } => "dangling_beat",
        Problem::DeletedBeat { .. } => "deleted_beat",
        Problem::UnknownScene { .. } => "unknown_scene",
        Problem::NoDestination => "no_destination",
        Problem::NoBeats => "no_beats",
        Problem::Type(_) => "type_error",
        Problem::NotAParticipant { .. } => "not_a_participant",
        Problem::UnknownSetting { .. } => "unknown_setting",
        Problem::MissingText => "missing_text",
        Problem::RevisionMismatch { .. } => "revision_mismatch",
        Problem::DuplicateId { .. } => "duplicate_id",
        Problem::NoTextEntries => "no_text_entries",
        Problem::ProseHasCast { .. } => "prose_has_cast",
        Problem::VoiceNotAllowed { .. } => "voice_not_allowed",
        Problem::WrongLineCount { .. } => "wrong_line_count",
    }
}

impl DiagnosticView {
    pub(super) fn of(diagnostic: &Diagnostic) -> DiagnosticView {
        let mut view = DiagnosticView {
            kind: "scene",
            code: problem_code(&diagnostic.problem),
            message: diagnostic.problem.to_string(),
            destination: diagnostic.problem.is_destination(),
            beat_id: None,
            choice_id: None,
            outcome_id: None,
            slot_id: None,
            variant_id: None,
            entity_id: None,
            intent_index: None,
            entry_id: None,
        };
        match diagnostic.site {
            Site::Scene => {}
            Site::Entry => view.kind = "entry",
            Site::Setting => {
                view.kind = "setting";
                if let Problem::UnknownSetting { id } = diagnostic.problem {
                    view.entity_id = Some(id);
                }
            }
            Site::Participant { entity } => {
                view.kind = "participant";
                view.entity_id = Some(entity);
            }
            Site::Destination(DestinationSite::Choice { beat, choice }) => {
                view.kind = "choice";
                view.beat_id = Some(beat);
                view.choice_id = Some(choice);
            }
            Site::Destination(DestinationSite::Outcome { beat, outcome }) => {
                view.kind = "outcome";
                view.beat_id = Some(beat);
                view.outcome_id = Some(outcome);
            }
            Site::Destination(DestinationSite::Beat(beat)) => {
                view.kind = "beat";
                view.beat_id = Some(beat);
            }
            Site::Intent { beat, index } => {
                view.kind = "intent";
                view.beat_id = Some(beat);
                view.intent_index = Some(index);
            }
            Site::DialogueSlot { beat, slot } => {
                view.kind = "dialogueSlot";
                view.beat_id = Some(beat);
                view.slot_id = Some(slot);
            }
            Site::Variant { beat, slot, variant } => {
                view.kind = "variant";
                view.beat_id = Some(beat);
                view.slot_id = Some(slot);
                view.variant_id = Some(variant);
            }
            Site::TextAsset => view.kind = "textAsset",
            Site::TextTrigger => view.kind = "textTrigger",
            Site::TextEntry { entry } => {
                view.kind = "textEntry";
                view.entry_id = Some(entry);
            }
            Site::TextSlot { entry, slot } => {
                view.kind = "textLine";
                view.entry_id = Some(entry);
                view.slot_id = Some(slot);
            }
            Site::TextVariant { entry, slot, variant } => {
                view.kind = "textVariant";
                view.entry_id = Some(entry);
                view.slot_id = Some(slot);
                view.variant_id = Some(variant);
            }
        }
        view
    }
}

/* ── layout ───────────────────────────────────────────────────────────────── */

/// Something the canvas should know and must not be stopped by.
///
/// `blocking` is read off `LayoutNotice::is_blocking` rather than hardcoded to
/// `false` here. It is false for every variant today; carrying the model's own
/// answer means that if a blocking variant is ever added, this surface reports
/// it instead of continuing to promise it cannot happen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutNoticeView {
    pub kind: &'static str,
    pub blocking: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub found: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported: Option<u32>,
    /// Node keys, in the `beat:<ulid>` spelling the layout file uses, so a
    /// notice can be matched to a canvas node without a second vocabulary.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<String>,
}

impl LayoutNoticeView {
    fn of(notice: &LayoutNotice) -> LayoutNoticeView {
        let blank = |kind: &'static str| LayoutNoticeView {
            kind,
            blocking: notice.is_blocking(),
            rel: None,
            reason: None,
            found: None,
            supported: None,
            nodes: Vec::new(),
        };
        match notice {
            LayoutNotice::Missing { rel } => {
                LayoutNoticeView { rel: Some(rel.clone()), ..blank("missing") }
            }
            LayoutNotice::Unreadable { rel, reason } => LayoutNoticeView {
                rel: Some(rel.clone()),
                reason: Some(reason.clone()),
                ..blank("unreadable")
            },
            LayoutNotice::NewerSchema { rel, found, supported } => LayoutNoticeView {
                rel: Some(rel.clone()),
                found: Some(*found),
                supported: Some(*supported),
                ..blank("newerSchema")
            },
            LayoutNotice::WrongGraph { rel } => {
                LayoutNoticeView { rel: Some(rel.clone()), ..blank("wrongGraph") }
            }
            LayoutNotice::Unplaced { nodes } => LayoutNoticeView {
                nodes: nodes.iter().map(ToString::to_string).collect(),
                ..blank("unplaced")
            },
            LayoutNotice::Stale { nodes } => LayoutNoticeView {
                nodes: nodes.iter().map(ToString::to_string).collect(),
                ..blank("stale")
            },
        }
    }
}

/// A layout and everything that was wrong with getting it.
///
/// The load stamp is deliberately not carried across the bridge. Layout writes
/// merge per node id rather than comparing a precondition, so a stamp here
/// would be a number the webview could only misuse — the first caller to treat
/// it as a guard would have invented a conflict the layout format has no way
/// to report.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutLoadView {
    pub layout: Layout,
    pub notices: Vec<LayoutNoticeView>,
}

impl LayoutLoadView {
    fn of(loaded: LayoutLoad) -> LayoutLoadView {
        LayoutLoadView {
            notices: loaded.notices.iter().map(LayoutNoticeView::of).collect(),
            layout: loaded.layout,
        }
    }
}

/// What happened to an arrangement.
///
/// An outcome rather than a `Result`, and that is the whole of "layout can
/// never stop anybody" at this layer. A read-only folder, an unmounted share,
/// a sidecar written by a newer Wobu and a disk that refused the write all
/// arrive here as something to mention, never as a rejected promise the caller
/// has to catch. A drag is not an operation a person should be able to fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum LayoutSaveView {
    Written,
    /// The file on disk was written by a newer Wobu. Not written, and nothing
    /// lost: overwriting would trade a colleague's whole arrangement for one
    /// drag.
    Deferred {
        rel: String,
        found: u32,
        supported: u32,
    },
    /// Nothing reached disk and the arrangement is only in this session.
    /// `reason` is the store's own wording.
    Unwritable {
        reason: String,
    },
}

/* ── scenes ───────────────────────────────────────────────────────────────── */

/// Every scene in the project, plus the files that could not be identified.
///
/// Read from the folder on every call. Scenes are not in the local index yet
/// (#153), so there is no cheaper answer that would also be a correct one, and
/// a stale scene list is the thing that makes a writer open a scene somebody
/// deleted.
#[tauri::command]
pub fn narrative_scenes(state: State<'_, AppState>) -> CommandResult<SceneCatalogView> {
    state.with(|project| scenes(project))
}

fn scenes(project: &Project) -> CommandResult<SceneCatalogView> {
    let catalog = project.scene_catalog()?;
    Ok(SceneCatalogView {
        scenes: catalog
            .scenes
            .iter()
            .map(|entry| SceneSummary {
                id: entry.id,
                name: entry.name.clone(),
                slug: entry.slug.clone(),
                rel: entry.rel.clone(),
            })
            .collect(),
        unreadable: catalog
            .unreadable
            .iter()
            .map(|bad| UnreadableScene { rel: bad.rel.clone(), reason: bad.reason.clone() })
            .collect(),
    })
}

/// One scene, whole, with the precondition for saving it back.
#[tauri::command]
pub async fn narrative_scene_get(
    app: AppHandle,
    scene_id: SceneId,
) -> CommandResult<SceneFileView> {
    let (ticket, ()) = app.state::<AppState>().ticket(|_| Ok(()))?;
    super::blocking("The scene read thread stopped unexpectedly.", move || {
        app.state::<AppState>()
            .with_ticket(&ticket, |project| Ok(SceneFileView::of(&project.load_scene(scene_id)?)))
    })
    .await?
}

#[tauri::command]
pub fn narrative_scene_create(
    state: State<'_, AppState>,
    name: String,
) -> CommandResult<SceneFileView> {
    state.with(|project| Ok(SceneFileView::of(&project.create_scene(&name)?)))
}

/// Change a scene's display name, and nothing else.
///
/// Its own command rather than a `narrative_scene_save` of a document the
/// caller does not hold: the Library row that offers Rename has a name and an
/// id, and making it fetch a whole scene to change a string would mean a
/// rename could fail because a beat elsewhere in the file did not parse.
///
/// The precondition is read and used inside one `AppState::with`, so no other
/// command on this machine can interleave. There is no stamp argument for the
/// same reason there is no stamp cache: the Library holds no version of this
/// file, and inventing one for it to send would be a precondition that is
/// stale by definition.
///
/// Nothing else moves. The slug, and therefore the file name, is minted once
/// at create — a rename that moved the file would read as a delete and a
/// create to every sync client, watcher and git history looking at the folder.
#[tauri::command]
pub fn narrative_scene_rename(
    state: State<'_, AppState>,
    scene_id: SceneId,
    name: String,
) -> CommandResult<SceneFileView> {
    state.with(|project| rename_scene(project, scene_id, name))
}

fn rename_scene(
    project: &mut Project,
    scene_id: SceneId,
    name: String,
) -> CommandResult<SceneFileView> {
    let mut file = project.load_scene(scene_id)?;
    file.scene.name = name;
    match project.save_scene(&mut file)? {
        SourceSave::Saved(_) => Ok(SceneFileView::of(&file)),
        SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}

/// Delete a scene and the arrangement that described it.
///
/// Scenes that link *to* it are deliberately not rewired; a destination
/// pointing at a removed scene becomes a diagnostic that names it, which is a
/// thing to undo, where a silent repair is not.
#[tauri::command]
pub fn narrative_scene_delete(state: State<'_, AppState>, scene_id: SceneId) -> CommandResult<()> {
    state.with(|project| Ok(project.delete_scene(scene_id)?))
}

/// Write a whole scene document.
///
/// The one write path for narrative source, and therefore the one place a
/// structural edit reaches disk however it was made — a canvas connection, a
/// form field, a reordered beat, a typed line. #156 requires a canvas edit and
/// the equivalent form edit to produce identical source; one command is how
/// that is guaranteed rather than tested for.
///
/// `slug` is only consulted for a scene the catalog does not know — undo
/// restoring one that was deleted. It is re-slugified rather than trusted,
/// because it becomes a file name on somebody's SMB share, and it is made
/// unique against the scenes that exist so a restore cannot land on top of a
/// different scene that took the name in the meantime. The restored scene is
/// still itself: the id, and every beat, choice and line id under it, are the
/// ones that were recorded.
#[tauri::command]
pub fn narrative_scene_save(
    state: State<'_, AppState>,
    scene: Scene,
    slug: Option<String>,
    expected: Precondition,
) -> CommandResult<SceneFileView> {
    state.with(|project| save_scene(project, scene, slug.as_deref(), &expected))
}

fn save_scene(
    project: &mut Project,
    scene: Scene,
    slug: Option<&str>,
    expected: &Precondition,
) -> CommandResult<SceneFileView> {
    if matches!(expected, Precondition::Current) {
        return Err(WobuError::new(
            Code::Invalid,
            "Scene saves require the original stamp. Undo uses a guarded document restore.",
        ));
    }
    let catalog = project.scene_catalog()?;
    let rel = match catalog.find(scene.id) {
        // The catalog is authoritative for a scene that exists. Taking the
        // path from the caller would let a stale editor tab write a scene to
        // wherever it last remembered the file being.
        Some(entry) => entry.rel.clone(),
        None => {
            let taken = catalog.slugs();
            let base = wobu_core::slugify(slug.unwrap_or(&scene.name))?;
            let unique = wobu_core::unique_slug(&base, &|candidate| taken.contains(candidate));
            wobu_store::narrative::scene_rel(&unique)
        }
    };

    // Read without parsing: the precondition is a fact about the bytes, and a
    // file that will not parse still has a stamp. Parsing here would mean a
    // scene somebody broke in a text editor could not be saved over even by
    // the person fixing it.
    let path = wobu_store::paths::from_rel_string(project.root(), &rel);
    let on_disk = wobu_store::atomic::read_stamped(&path)?;
    let on_disk = on_disk.map(|(_, stamp)| stamp);

    let mut file = wobu_store::SceneFile { scene, rel, stamp: expected.against(on_disk) };
    match project.save_scene(&mut file)? {
        SourceSave::Saved(_) => Ok(SceneFileView::of(&file)),
        SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}

/* ── declared state ───────────────────────────────────────────────────────── */

/// The declared variables, or an empty document in a project that has none.
///
/// An absent file is not an error. A project can have scenes long before it
/// has state, and a Narrative workspace that refused to open until somebody
/// declared a variable would be a workspace nobody could start using.
#[tauri::command]
pub fn narrative_state_get(state: State<'_, AppState>) -> CommandResult<StateFileView> {
    state.with(|project| {
        Ok(match project.state_document()? {
            Some((document, stamp)) => StateFileView { document, stamp: Some(stamp) },
            None => StateFileView { document: StateDocument::new(Vec::new()), stamp: None },
        })
    })
}

/// Write the declared variables.
///
/// Checked before it is written, and refused if it does not hold together on
/// its own terms — a name declared twice, a default outside its own range.
/// This is not the same policy as a scene, which is saved half-finished all
/// day long, and the difference is that a scene's problems are reported
/// against the scene while a broken schema makes *every* scene's diagnostics
/// wrong at once. There is nowhere to show that, so it is refused where the
/// author can still see what they typed.
#[tauri::command]
pub fn narrative_state_save(
    state: State<'_, AppState>,
    document: StateDocument,
    expected: Precondition,
) -> CommandResult<StateFileView> {
    state.with(|project| save_state(project, document, &expected))
}

pub(super) fn save_state(
    project: &mut Project,
    document: StateDocument,
    expected: &Precondition,
) -> CommandResult<StateFileView> {
    document.schema().map_err(|error| {
        WobuError::new(Code::Invalid, "These variables do not hold together.")
            .with_detail(error.to_string())
    })?;

    let on_disk = project.state_document()?.map(|(_, stamp)| stamp);
    let expected = expected.against(on_disk);
    match project.save_state(&document, expected.as_ref())? {
        SourceSave::Saved(stamp) => Ok(StateFileView { document, stamp: Some(stamp) }),
        SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
    }
}

/* ── diagnostics ──────────────────────────────────────────────────────────── */

/// What is wrong with one scene, keyed by the element responsible.
///
/// `scene` overrides what is on disk, and it is the whole reason this is one
/// command rather than two. The canvas and the Script tab both hold a document
/// with unsaved edits in it; a diagnostics call that could only read the saved
/// file would mark a destination the writer connected thirty seconds ago as
/// still broken, and they would learn to ignore the list. Passing `null` reads
/// the saved scene, which is what a project-wide Validation list wants.
///
/// The scene catalog and the state schema always come from the project, so a
/// cross-scene link is checked against the scenes that really exist rather
/// than against anything the caller asserted.
#[tauri::command]
pub fn narrative_diagnostics(
    state: State<'_, AppState>,
    scene_id: SceneId,
    scene: Option<Scene>,
) -> CommandResult<Vec<DiagnosticView>> {
    state.with(|project| diagnostics(project, scene_id, scene))
}

pub(crate) fn diagnostics(
    project: &Project,
    scene_id: SceneId,
    scene: Option<Scene>,
) -> CommandResult<Vec<DiagnosticView>> {
    let scene = match scene {
        Some(scene) => scene,
        None => project.load_scene(scene_id)?.scene,
    };
    Ok(diagnose(&scene, &project_context(project)?))
}

/// The project-wide inputs every scene is checked against.
///
/// Read once and reused rather than re-read per scene. A caller checking one
/// scene pays for one read either way; a caller checking three hundred of them
/// — `narrative_diagnostics` over MCP is the one that does — would otherwise
/// re-scan the scene folder and re-read the state and world files once per
/// scene, which is nine hundred file operations to answer one question.
pub(crate) struct DiagnosticContext {
    schema: wobu_narrative::StateSchema,
    catalog: SceneCatalog,
    world: wobu_narrative::WorldDocument,
    /// The project's `setting` nodes, for the scene's stated place (#206). Read
    /// from the index as summaries rather than node by node: the kind is all this
    /// needs, and a caller checking three hundred scenes must not pay a file read
    /// per node per scene.
    settings: std::collections::BTreeSet<wobu_core::Id>,
}

pub(crate) fn project_context(project: &Project) -> CommandResult<DiagnosticContext> {
    Ok(DiagnosticContext {
        schema: project.state_schema()?,
        // Always the project's, never the caller's: a cross-scene link is
        // checked against the scenes that really exist rather than against
        // anything the caller asserted.
        catalog: SceneCatalog::of(project.scene_ids()?),
        world: project.world_document()?.map(|(document, _)| document).unwrap_or_default(),
        settings: setting_ids(project)?,
    })
}

/// The ids of every `setting` node in the project.
pub(crate) fn setting_ids(
    project: &Project,
) -> CommandResult<std::collections::BTreeSet<wobu_core::Id>> {
    Ok(project
        .list_nodes()?
        .into_iter()
        .filter(|node| node.kind == wobu_core::NodeKind::Setting)
        .map(|node| node.id)
        .collect())
}

pub(crate) fn diagnose(scene: &Scene, context: &DiagnosticContext) -> Vec<DiagnosticView> {
    scene
        .diagnostics(&context.schema, &context.catalog)
        .into_iter()
        .chain(
            scene
                .classification_diagnostics(&context.world)
                .into_iter()
                .chain(scene.setting_diagnostics(&context.settings))
                .map(|(diagnostic, _)| diagnostic),
        )
        .map(|diagnostic| DiagnosticView::of(&diagnostic))
        .collect()
}

/* ── layout ───────────────────────────────────────────────────────────────── */

/// Where the boxes are for one graph.
///
/// Reconciled against the source as it is read — positions for ids the scene
/// no longer has are dropped, ids with no position are listed — but nothing is
/// written by the reading. A project opened to be looked at, or a read-only
/// share, must not be rewritten by the looking.
#[tauri::command]
pub fn narrative_layout_get(
    state: State<'_, AppState>,
    graph: GraphKey,
) -> CommandResult<LayoutLoadView> {
    state.with(|project| Ok(layout_get(project, &graph)))
}

fn layout_get(project: &Project, graph: &GraphKey) -> LayoutLoadView {
    match graph {
        GraphKey::Scene { scene } => match project.load_scene(*scene) {
            Ok(file) => LayoutLoadView::of(project.scene_layout(&file.scene)),
            // A scene that will not load still has an arrangement, and the
            // Source tab is exactly where somebody goes to repair it. Loading
            // the raw sidecar without reconciling is the honest answer: we
            // cannot say which ids are stale without a document to compare to.
            Err(_) => {
                LayoutLoadView::of(wobu_store::narrative::layout::load(project.root(), graph))
            }
        },
        GraphKey::Arc { arc } => LayoutLoadView::of(project.arc_layout(arc)),
        GraphKey::Quest { quest } => LayoutLoadView::of(project.quest_layout(*quest)),
    }
}

/// Save an arrangement. Cannot fail, by construction.
///
/// Every failure is an outcome rather than an error. That is not politeness:
/// an error here would reach `report()` and put a toast on screen, and a canvas
/// autosaving positions produces one of these per drag. A person who has just
/// tidied forty nodes on a read-only share would get forty toasts telling them
/// something they can already see in the banner.
///
/// Nothing on this path can affect a source save. They are different files
/// behind different commands with no shared state — so the guarantee holds
/// because there is nothing to get wrong, not because a caller sequenced them
/// carefully.
#[tauri::command]
pub fn narrative_layout_save(
    state: State<'_, AppState>,
    layout: Layout,
) -> CommandResult<LayoutSaveView> {
    state.with(|project| Ok(layout_save(project, &layout)))
}

fn layout_save(project: &Project, layout: &Layout) -> LayoutSaveView {
    let saved = match &layout.graph {
        GraphKey::Scene { scene } => match project.load_scene(*scene) {
            // The scene is handed in so the write can prune positions for ids
            // it no longer contains. Without a scene we decline rather than
            // saving unpruned: an arrangement that keeps growing entries for
            // deleted beats is the thing `sweep` exists to clean up, and
            // adding to it here would be making the mess on purpose.
            Ok(file) => project.save_scene_layout(&file.scene, layout),
            Err(error) => return LayoutSaveView::Unwritable { reason: error.to_string() },
        },
        GraphKey::Arc { .. } => project.save_arc_layout(layout),
        GraphKey::Quest { quest } => project.save_quest_layout(layout, *quest),
    };
    match saved {
        Ok(LayoutSave::Written(_)) => LayoutSaveView::Written,
        Ok(LayoutSave::Deferred { rel, found, supported }) => {
            LayoutSaveView::Deferred { rel, found, supported }
        }
        Err(error) => LayoutSaveView::Unwritable { reason: error.to_string() },
    }
}

#[cfg(test)]
pub(super) mod tests;

/// Prepare handwritten wording without credentials or a generation job.
#[tauri::command]
pub fn narrative_text_written(body: String, locked: bool) -> wobu_narrative::Text {
    if locked {
        wobu_narrative::Text::written_locked(body)
    } else {
        wobu_narrative::Text::written(body)
    }
}

#[cfg(test)]
#[path = "narrative/ashfall_tests.rs"]
mod ashfall_tests;
