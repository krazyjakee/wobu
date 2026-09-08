//! Coherent frozen generation input, shared by inspection and guarded publication.
use crate::{Error, Project, Result, atomic::Stamp};
use std::collections::{BTreeMap, BTreeSet};
use wobu_core::NodeKind;
use wobu_narrative::{EntityId, KnowledgeProvenance, WorldDocument};
use wobu_narrative_context::{Character, FrozenContext, Input, Options, resolve};
fn invalid(message: impl Into<String>) -> Error {
    Error::Malformed { path: "narrative/context".into(), reason: message.into() }
}
pub fn capture(
    project: &Project,
    options: Options,
    after_read: impl FnOnce(),
) -> Result<FrozenContext> {
    let fingerprint = project.narrative_fingerprint()?;
    let file = project.load_scene(options.selection.scene)?;
    let state = project.state_document()?;
    let schema = state
        .as_ref()
        .map(|(document, _)| document.schema())
        .transpose()
        .map_err(|error| invalid(error.to_string()))?
        .unwrap_or_default();
    let world_file = project.world_document()?;
    let world = world_file.as_ref().map(|(document, _)| document).cloned().unwrap_or_default();
    let character_ids = referenced_characters(&file.scene, &world, &options);
    let characters = read_characters(project, &character_ids)?;
    after_read();
    let current_scene =
        crate::atomic::read_stamped(&project.root().join(&file.rel))?.map(|(_, stamp)| stamp);
    let state_stamp = state.as_ref().map(|(_, stamp)| stamp);
    let world_stamp = world_file.as_ref().map(|(_, stamp)| stamp);
    if current_scene != file.stamp
        || project.state_document()?.as_ref().map(|(_, stamp)| stamp) != state_stamp
        || project.world_document()?.as_ref().map(|(_, stamp)| stamp) != world_stamp
        || read_characters(project, &character_ids)? != characters
        || project.narrative_fingerprint()? != fingerprint
    {
        return Err(invalid(
            "Source changed while capturing context. Save or reload, then inspect again.",
        ));
    }
    let characters = characters
        .into_iter()
        .filter_map(|(id, (character, _))| character.map(|character| (id, character)))
        .collect();
    let result = resolve(
        Input { scene: &file.scene, world: &world, schema: &schema, characters: &characters },
        options,
    );
    Ok(result)
}
fn referenced_characters(
    scene: &wobu_narrative::Scene,
    world: &WorldDocument,
    options: &Options,
) -> BTreeSet<EntityId> {
    let speaker = scene
        .beats
        .iter()
        .find(|b| b.id == options.selection.beat)
        .and_then(|b| b.dialogue.iter().find(|s| s.id == options.selection.slot))
        .and_then(|slot| slot.speaker.entity());
    scene
        .participants
        .iter()
        .map(|p| p.entity)
        .chain(speaker)
        .chain(world.knowledge.iter().filter(|k| Some(k.character) == speaker).filter_map(|k| {
            match k.provenance {
                KnowledgeProvenance::Told { by } => Some(by),
                _ => None,
            }
        }))
        .collect()
}
type CharacterCapture = BTreeMap<EntityId, (Option<Character>, Option<Stamp>)>;
fn read_characters(project: &Project, ids: &BTreeSet<EntityId>) -> Result<CharacterCapture> {
    ids.iter()
        .map(|id| match project.get_node_stamped(*id) {
            Ok((node, stamp)) => {
                if node.id != *id {
                    return Err(invalid(
                        "Character identity changed. Reload before inspecting context.",
                    ));
                }
                let character = if node.kind == NodeKind::Character {
                    let voice = match node.attributes.get("narrative_voice") {
                        None => None,
                        Some(serde_json::Value::String(value)) => Some(value.clone()),
                        Some(_) => {
                            return Err(invalid(format!(
                                "Character {} has a non-text narrative_voice attribute.",
                                node.name
                            )));
                        }
                    };
                    Some(Character { id: *id, name: node.name, voice })
                } else {
                    None
                };
                Ok((*id, (character, Some(stamp))))
            }
            Err(crate::Error::NoSuchNode(_)) => Ok((*id, (None, None))),
            Err(error) => Err(error.into()),
        })
        .collect()
}
