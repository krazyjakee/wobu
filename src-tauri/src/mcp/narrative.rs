//! The authored story, as much of it as an agent may reach.
//!
//! The other half of `mcp.rs`'s [`ProjectWorld`](super::ProjectWorld): where
//! that file answers questions about entities and the influence between them,
//! this one answers questions about scenes, beats, lines and canon. It is a
//! separate file for the same reason [`Narrative`] is a separate trait — the
//! two bodies of source are two, with two file formats and two sets of rules
//! about what may be written — and the rules here are the stricter ones.
//!
//! ## What an agent may write into a story, and what it may not
//!
//! Two tools, both additive. `create_scene` writes a new file and touches
//! nothing that exists. `draft_dialogue` fills a dialogue slot that has no
//! words in it yet, and is refused for a slot that has any or that is locked.
//!
//! There is deliberately no whole-document write. A scene save is guarded by
//! the [`Stamp`](wobu_store::atomic::Stamp) the reader held, which is what
//! stops two writers clobbering each other on a shared folder; an agent posting
//! one stateless request at a time holds no such thing, and inventing one for
//! it — reading the file inside the write and saving over whatever is there —
//! would turn a detected conflict into a silent overwrite of somebody's
//! afternoon. `draft_dialogue` reads and writes inside one `AppState::with`, so
//! its precondition is the one it actually observed.
//!
//! And nothing here can author a branch. `draft_dialogue` writes one
//! unconditional wording into a slot a person already made; it cannot add a
//! beat, a choice, an outcome, an effect or a condition. That is the same line
//! #151 draws around generation, held for the same reason: prose is inert, and
//! where the story goes is the writer's statement.
//!
//! ## Why the wording is `Imported` and never `Human`
//!
//! [`Provenance`] is half of what a revision hashes, and the review queue reads
//! it to decide what it is looking at. `Human` would tell a reviewer a person
//! typed a line nobody in the project has read. `Generated` would claim a
//! generation receipt that does not exist — there is no job, no model and no
//! fingerprint behind an MCP call. `Imported` is the honest third answer, and
//! it is what that variant is for: wording that came from outside Wobu.

use serde_json::{Value, json};
use wobu_mcp::world::{Narrative, SceneFilter, WorldResult};
use wobu_narrative::{
    ContentLifecycle, DialogueSlotId, GenerationPolicy, Provenance, Revision, Scene, SceneId, Text,
    TextAssetId, Variant,
};
use wobu_store::SourceSave;
use wobu_store::project::narrative_library::{LibraryQuery, QueryError};

use super::{ProjectWorld, value};
use crate::error::{Code, CommandResult, WobuError};

/// How many scenes a project-wide diagnostics call will name.
///
/// The count is exact — every scene is read and every problem is counted — and
/// only the list is cut, with `truncated` saying so. A model that is handed
/// three hundred scenes' worth of problems reads none of them, and a total that
/// quietly disagreed with the list would be worse than either.
const DIAGNOSTIC_SCENE_LIMIT: usize = 50;

fn scene_id(raw: &str) -> CommandResult<SceneId> {
    raw.trim()
        .parse()
        .map_err(|_| WobuError::new(Code::Invalid, format!("{raw:?} is not a scene id (a ULID).")))
}

fn slot_id(raw: &str) -> CommandResult<DialogueSlotId> {
    raw.trim().parse().map_err(|_| {
        WobuError::new(Code::Invalid, format!("{raw:?} is not a dialogue slot id (a ULID)."))
    })
}

fn asset_id(raw: &str) -> CommandResult<TextAssetId> {
    raw.trim().parse().map_err(|_| {
        WobuError::new(Code::Invalid, format!("{raw:?} is not a text asset id (a ULID)."))
    })
}

/// The library query behind `list_scenes`, `search_narrative` and the first
/// page of `wobu://scenes`.
///
/// `include_drafts` is on, unlike the pane's default. A writer searching their
/// own project is usually looking for shippable text; an agent asked to find
/// where something is said needs to be shown the unapproved lines too, and a
/// search that silently skipped them would have it conclude the line is not
/// there and write a second one.
fn library_query(filter: &SceneFilter) -> LibraryQuery {
    LibraryQuery {
        query: filter.query.clone(),
        participant: filter.participant.clone(),
        quest: filter.quest.clone(),
        act: filter.act.clone(),
        arc: filter.arc.clone(),
        tag: filter.tag.clone(),
        review: filter.review.clone(),
        freshness: filter.freshness.clone(),
        missing: filter.missing_text,
        include_drafts: true,
        offset: filter.offset,
        limit: filter.limit.clamp(1, 100),
        ..LibraryQuery::default()
    }
}

fn query_error(error: QueryError) -> WobuError {
    match error {
        QueryError::StaleRevision => WobuError::new(Code::Conflict, error.to_string()),
        QueryError::Invalid(_) => WobuError::new(Code::Invalid, error.to_string()),
        QueryError::Store(error) => error.into(),
    }
}

impl Narrative for ProjectWorld {
    fn overview(&self) -> WorldResult {
        self.with(|project| {
            // One row of each query: what is wanted is the totals and the
            // facets, which the library computes whatever the page size, and a
            // hundred scene rows in an overview would bury them.
            let page = project
                .library_query(&LibraryQuery { limit: 1, ..LibraryQuery::default() })
                .map_err(query_error)?;
            let unfilled = project
                .library_query(&LibraryQuery { limit: 1, missing: true, ..LibraryQuery::default() })
                .map_err(query_error)?;
            let texts = project.text_catalog()?;
            let world = project.world_document()?.map(|(document, _)| document).unwrap_or_default();
            let variables = project
                .state_document()?
                .map(|(document, _)| document.variables.len())
                .unwrap_or_default();

            Ok(json!({
                "sceneCount": page.scene_count,
                "scenesMissingText": unfilled.total,
                "unreadableScenes": page.unreadable_total,
                "textAssetCount": texts.assets.len(),
                "declaredVariables": variables,
                "classifications": value(&page.facets),
                "canonCounts": {
                    "facts": world.facts.len(),
                    "knowledge": world.knowledge.len(),
                    "relationships": world.relationships.len(),
                    "events": world.events.len(),
                    "quests": world.quests.len(),
                    "restrictions": world.restrictions.len(),
                },
                "note": "A scene's participants are world-model characters. Their ids are the \
                         ones list_nodes reports.",
            }))
        })
    }

    fn scenes(&self, filter: &SceneFilter) -> WorldResult {
        self.with(|project| {
            let page = project.library_query(&library_query(filter)).map_err(query_error)?;
            Ok(value(&page))
        })
    }

    fn scene(&self, id: &str) -> WorldResult {
        self.with(|project| {
            let file = project.load_scene(scene_id(id)?)?;
            // No stamp. It is the precondition for a save this surface does not
            // offer, and handing an agent one would suggest otherwise.
            Ok(json!({ "rel": file.rel, "scene": value(&file.scene) }))
        })
    }

    fn declared_state(&self) -> WorldResult {
        self.with(|project| {
            // An absent file is not an error, exactly as it is not one for the
            // Narrative workspace: a project can have scenes long before it has
            // declared a variable.
            let document = project.state_document()?.map(|(document, _)| document);
            Ok(json!({
                "declared": document.is_some(),
                "variables": document.map(|document| value(&document.variables)),
            }))
        })
    }

    fn canon(&self) -> WorldResult {
        self.with(|project| Ok(value(&crate::commands::narrative_world::get(project)?)))
    }

    fn text_assets(&self) -> WorldResult {
        self.with(|project| {
            let catalog = project.text_catalog()?;
            let assets: Vec<Value> = catalog
                .assets
                .iter()
                .map(|entry| {
                    json!({
                        "assetId": entry.id.to_string(),
                        "kind": value(&entry.kind),
                        "name": entry.name,
                        "rel": entry.rel,
                    })
                })
                .collect();
            Ok(json!({
                "count": assets.len(),
                "assets": assets,
                "unreadable": value(&catalog.unreadable),
            }))
        })
    }

    fn text_asset(&self, id: &str) -> WorldResult {
        self.with(|project| {
            let file = project.load_text_asset(asset_id(id)?)?;
            Ok(json!({ "rel": file.rel, "asset": value(&file.asset) }))
        })
    }

    fn diagnostics(&self, scene: Option<&str>) -> WorldResult {
        self.with(|project| match scene {
            Some(id) => {
                let id = scene_id(id)?;
                let found = crate::commands::narrative::diagnostics(project, id, None)?;
                Ok(json!({
                    "scope": "scene",
                    "sceneId": id.to_string(),
                    "problemCount": found.len(),
                    "diagnostics": value(&found),
                }))
            }
            None => project_diagnostics(project),
        })
    }

    /* ── writes ───────────────────────────────────────────────────────────── */

    fn create_scene(&self, name: &str) -> WorldResult {
        self.with(|project| {
            let file = project.create_scene(name)?;
            Ok(json!({
                "sceneId": file.scene.id.to_string(),
                "name": file.scene.name,
                "rel": file.rel,
                "note": "Empty. Beats, choices and outcomes are the writer's to author.",
            }))
        })
    }

    fn draft_dialogue(&self, scene: &str, slot: &str, body: &str) -> WorldResult {
        self.with(|project| {
            let scene = scene_id(scene)?;
            let slot = slot_id(slot)?;
            let body = body.trim();
            if body.is_empty() {
                return Err(WobuError::new(Code::Invalid, "A line needs words in it."));
            }

            // Read and written inside one `AppState::with`, so the stamp this
            // save presents is the one this read observed and no other command
            // in this process can interleave between them.
            let mut file = project.load_scene(scene)?;
            let variant = draft_into(&mut file.scene, slot, body)?;
            match project.save_scene(&mut file)? {
                SourceSave::Saved(_) => Ok(variant),
                SourceSave::Conflict { conflict_path } => Err(WobuError::conflict(conflict_path)),
            }
        })
    }
}

/// Put one wording into an empty slot, or say why not.
///
/// Split out from the save so the refusals are readable in one place: an agent
/// is going to act on these sentences, and "that slot already has a wording"
/// has to be distinguishable from "there is no such slot" by something other
/// than tone.
fn draft_into(scene: &mut Scene, slot: DialogueSlotId, body: &str) -> CommandResult<Value> {
    let (beat_id, beat_title, slot) = scene
        .beats
        .iter_mut()
        .find_map(|beat| {
            let (id, title) = (beat.id, beat.title.clone());
            beat.dialogue.iter_mut().find(|d| d.id == slot).map(|d| (id, title, d))
        })
        .ok_or_else(|| {
            WobuError::new(
                Code::Invalid,
                "There is no dialogue slot with that id in this scene. get_scene lists them.",
            )
        })?;

    if slot.policy == GenerationPolicy::Locked {
        return Err(WobuError::new(
            Code::Invalid,
            "That slot is locked, which means nothing but the writer may put words in it.",
        ));
    }
    if !slot.variants.is_empty() {
        return Err(WobuError::new(
            Code::Invalid,
            "That slot already has a wording, and nothing here replaces one somebody wrote.",
        ));
    }

    let provenance = Provenance::Imported { source: MCP_SOURCE.to_owned() };
    let variant = Variant::new(Text {
        revision: Revision::of(body, &provenance),
        body: body.to_owned(),
        provenance,
        // The cautious default: `Edited`, so a later generation job may propose
        // a replacement beside this line but may not overwrite it, and `Draft`,
        // because nobody has read it.
        lifecycle: ContentLifecycle::default(),
    });
    let written = json!({
        "sceneId": scene.id.to_string(),
        "beatId": beat_id.to_string(),
        "beatTitle": beat_title,
        "slotId": slot.id.to_string(),
        "variantId": variant.id.to_string(),
        "revision": value(&variant.text.revision),
        "provenance": value(&variant.text.provenance),
        "note": "Written as an unreviewed draft from outside Wobu. It is in the review queue, \
                 not in a build.",
    });
    slot.variants.push(variant);
    Ok(written)
}

/// What the whole project's diagnostics say, at the cost of reading every scene.
///
/// The state schema, the scene catalog and the world document are read once for
/// the whole sweep rather than once per scene. Every scene's *source* still has
/// to be read — there is no cheaper answer that is also a correct one — but the
/// three things they are all checked against are the same three things.
fn project_diagnostics(project: &wobu_store::Project) -> CommandResult<Value> {
    let catalog = project.scene_catalog()?;
    let context = crate::commands::narrative::project_context(project)?;
    let mut listed: Vec<Value> = Vec::new();
    let mut problems = 0;
    let mut affected = 0;

    for entry in &catalog.scenes {
        let scene = project.load_scene(entry.id)?.scene;
        let found = crate::commands::narrative::diagnose(&scene, &context);
        if found.is_empty() {
            continue;
        }
        affected += 1;
        problems += found.len();
        if listed.len() < DIAGNOSTIC_SCENE_LIMIT {
            listed.push(json!({
                "sceneId": entry.id.to_string(),
                "name": entry.name,
                "diagnostics": value(&found),
            }));
        }
    }

    Ok(json!({
        "scope": "project",
        "sceneCount": catalog.scenes.len(),
        "scenesWithProblems": affected,
        "problemCount": problems,
        "truncated": affected > listed.len(),
        "scenes": listed,
        // A file that will not parse has no diagnostics because it has no
        // scene, and reporting nothing for it would read as "this is fine".
        "unreadable": value(&catalog.unreadable),
    }))
}

/// What the file records as the origin of an MCP-written line. Short and
/// stable: it is hashed into every revision written this way, so changing it
/// would change the identity of wording nobody edited.
const MCP_SOURCE: &str = "mcp";

#[cfg(test)]
mod tests {
    use super::*;
    use wobu_narrative::{Beat, DialogueSlot, ReviewState, Speaker};

    /// One beat, one empty slot, and the slot's id.
    fn scene_with_an_empty_slot() -> (Scene, DialogueSlotId) {
        let mut beat = Beat::new("Present evidence");
        beat.dialogue.push(DialogueSlot::new(Speaker::Narrator));
        let slot = beat.dialogue[0].id;
        let mut scene = Scene::new("The council hearing");
        scene.beats.push(beat);
        (scene, slot)
    }

    #[test]
    fn a_line_written_this_way_is_recorded_as_coming_from_outside_wobu() {
        let (mut scene, slot) = scene_with_an_empty_slot();
        let written = draft_into(&mut scene, slot, "The ridge is burning.").unwrap();

        let text = &scene.beats[0].dialogue[0].variants[0].text;
        assert_eq!(text.body, "The ridge is burning.");
        // Not `Human`: a reviewer must not be told a person typed this. Not
        // `Generated` either — there is no job and no fingerprint behind it.
        assert_eq!(text.provenance, Provenance::Imported { source: MCP_SOURCE.to_owned() });
        assert_eq!(text.lifecycle.review, ReviewState::Draft);
        assert_eq!(text.lifecycle.policy, GenerationPolicy::Edited);
        assert!(text.revision_matches(), "the stored revision must describe the stored words");
        assert_eq!(written["slotId"], slot.to_string());
    }

    #[test]
    fn nothing_written_this_way_can_change_where_the_story_goes() {
        // The line #151 draws around generation, held here for the same reason.
        // One unconditional wording, and no branch: no condition on the variant,
        // and no choice or outcome invented to hang it on.
        let (mut scene, slot) = scene_with_an_empty_slot();
        draft_into(&mut scene, slot, "The ridge is burning.").unwrap();

        assert!(scene.beats[0].dialogue[0].variants[0].when.is_none());
        assert!(scene.beats[0].choices.is_empty());
        assert!(scene.beats[0].outcomes.is_empty());
        assert_eq!(scene.beats.len(), 1);
    }

    #[test]
    fn a_slot_that_already_has_a_wording_is_left_exactly_as_it_was() {
        let (mut scene, slot) = scene_with_an_empty_slot();
        scene.beats[0].dialogue[0].variants.push(Variant::new(Text::written("Mine.")));
        let before = scene.clone();

        let error = draft_into(&mut scene, slot, "Not mine.").unwrap_err();
        assert!(error.message.contains("already has a wording"), "{}", error.message);
        assert_eq!(scene, before, "a refused draft still changed the scene");
    }

    #[test]
    fn a_locked_slot_is_refused_even_though_it_is_empty() {
        // US-05's guarantee, for a caller that never went near the UI: locking
        // stops anything but the writer putting words there.
        let (mut scene, slot) = scene_with_an_empty_slot();
        scene.beats[0].dialogue[0].policy = GenerationPolicy::Locked;

        let error = draft_into(&mut scene, slot, "The ridge is burning.").unwrap_err();
        assert!(error.message.contains("locked"), "{}", error.message);
        assert!(scene.beats[0].dialogue[0].variants.is_empty());
    }

    #[test]
    fn a_slot_id_this_scene_does_not_have_is_a_refusal_that_says_where_to_look() {
        let (mut scene, _) = scene_with_an_empty_slot();
        let error = draft_into(&mut scene, DialogueSlotId::new(), "Words.").unwrap_err();
        assert!(error.message.contains("get_scene"), "{}", error.message);
    }

    #[test]
    fn a_scene_id_that_is_not_a_ulid_is_reported_as_the_value_that_was_sent() {
        let error = scene_id("beat-3").unwrap_err();
        assert!(error.message.contains("beat-3"), "{}", error.message);
    }

    #[test]
    fn the_filter_reaches_the_library_as_the_library_spells_it() {
        let filter = SceneFilter {
            query: "logbook".into(),
            act: "01J".into(),
            missing_text: true,
            limit: 10_000,
            ..SceneFilter::default()
        };
        let query = library_query(&filter);

        assert_eq!(query.query, "logbook");
        assert_eq!(query.act, "01J");
        assert!(query.missing);
        // Unapproved lines are in scope for an agent, unlike the pane's default.
        assert!(query.include_drafts);
        assert_eq!(query.limit, 100, "a page size has to be a bound, not a suggestion");
    }
}
