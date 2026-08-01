//! Resolving several entity stacks into one deterministic scene stack.
//!
//! Shared upstream nodes are emitted once and use their strongest path. That is
//! deliberately `max`, never a sum: adding a second actor who shares the house
//! style must not make that style twice as loud. Entity sources remain ordered
//! as the caller named them so prompt clauses and immutable receipts agree on
//! which visual identity is which.

use std::collections::{BTreeMap, BTreeSet};

use wobu_core::{FragmentTarget, Id, Layer, default_preset, preset};

use crate::{
    Fragment, FragmentBody, Origin, Reached, ResolvedSource, ResolvedStack, Shot, Sliders, World,
    fragments, resolve,
};

pub const SCENE_FRAMING: &str =
    "multi-subject scene composition, keep every named entity visually distinct and recognizable";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneError {
    TooFewSubjects,
    DuplicateSubject(Id),
    MissingSubject(Id),
}

/// Who one merged source describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneScope {
    /// A root or upstream node reached by more than one entity.
    Shared,
    /// Context reached only from this entity's stack.
    Subject(Id),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedScene<'a> {
    subjects: Vec<Id>,
    stack: ResolvedStack<'a>,
    scopes: Vec<SceneScope>,
}

impl<'a> ResolvedScene<'a> {
    pub fn subjects(&self) -> &[Id] {
        &self.subjects
    }

    pub fn stack(&self) -> &ResolvedStack<'a> {
        &self.stack
    }

    /// Aligned one-for-one with [`ResolvedStack::sources`].
    pub fn scopes(&self) -> &[SceneScope] {
        &self.scopes
    }

    pub fn scope_for_node(&self, id: Id) -> Option<SceneScope> {
        self.stack
            .sources()
            .iter()
            .position(|candidate| candidate.node_id() == Some(id))
            .and_then(|index| self.scopes.get(index).copied())
    }
}

/// Extract a merged scene with each entity weighted by its own default preset.
///
/// Shared sources can appear in several independently resolved stacks. Their
/// exact fragments are emitted once at the greatest offered weight, matching
/// the merged source's max-path rule. Entity palettes remain separate because
/// their source node ids differ; the scene compiler labels those clauses by
/// entity instead of averaging incompatible colours into a fictitious palette.
pub fn scene_fragments<'a>(world: &World<'a>, scene: &ResolvedScene<'a>) -> Vec<Fragment<'a>> {
    let mut offered = Vec::new();
    for subject in scene.subjects() {
        let Some(node) = world.get(*subject) else { continue };
        let Some(stack) = resolve(world, *subject, None) else { continue };
        offered.push(fragments(&stack, default_preset(node.kind), &Sliders::neutral()));
    }

    let mut out: Vec<Fragment<'a>> = Vec::new();
    for source in scene.stack().sources() {
        let Some(node_id) = source.node_id() else {
            let framing = preset("environment_matte")
                .map(|preset| preset.framing)
                .unwrap_or("wide establishing composition, coherent light and perspective");
            out.push(Fragment::new(
                source,
                "framing",
                FragmentBody::Text(framing),
                source.weight,
                FragmentTarget::Prompt,
            ));
            out.push(Fragment::new(
                source,
                "scene_identity",
                FragmentBody::Text(SCENE_FRAMING),
                source.weight,
                FragmentTarget::Prompt,
            ));
            continue;
        };
        for fragment in offered
            .iter()
            .flat_map(|fragments| fragments.iter().copied())
            .filter(|fragment| fragment.node_id() == Some(node_id))
        {
            let duplicate = out.iter().position(|existing| {
                existing.node_id() == Some(node_id)
                    && existing.section() == fragment.section()
                    && existing.body() == fragment.body()
                    && existing.target() == fragment.target()
            });
            if let Some(index) = duplicate {
                if fragment.weight() > out[index].weight() {
                    out[index] = Fragment::new(
                        source,
                        fragment.section(),
                        fragment.body(),
                        fragment.weight(),
                        fragment.target(),
                    );
                }
                continue;
            }
            out.push(Fragment::new(
                source,
                fragment.section(),
                fragment.body(),
                fragment.weight(),
                fragment.target(),
            ));
        }
    }
    out
}

/// Merge independently resolved stacks into one scene.
pub fn resolve_scene<'a>(
    world: &World<'a>,
    subjects: &[Id],
    shot: Shot<'a>,
) -> Result<ResolvedScene<'a>, SceneError> {
    if subjects.len() < 2 {
        return Err(SceneError::TooFewSubjects);
    }
    let mut distinct = BTreeSet::new();
    for id in subjects {
        if !distinct.insert(*id) {
            return Err(SceneError::DuplicateSubject(*id));
        }
        if world.get(*id).is_none() {
            return Err(SceneError::MissingSubject(*id));
        }
    }

    let stacks: Vec<ResolvedStack<'a>> = subjects
        .iter()
        .map(|id| resolve(world, *id, None).ok_or(SceneError::MissingSubject(*id)))
        .collect::<Result<_, _>>()?;
    let selected: BTreeSet<Id> = subjects.iter().copied().collect();
    let mut owners: BTreeMap<Id, BTreeSet<Id>> = BTreeMap::new();
    for (subject, stack) in subjects.iter().zip(&stacks) {
        for source in stack.sources() {
            if let Some(id) = source.node_id() {
                owners.entry(id).or_default().insert(*subject);
            }
        }
    }

    let mut sources: Vec<ResolvedSource<'a>> = Vec::new();
    let mut scopes = Vec::new();
    let layers =
        [Layer::Style, Layer::World, Layer::Ancestry, Layer::Culture, Layer::Place, Layer::Subject];
    for layer in layers {
        for (subject, stack) in subjects.iter().zip(&stacks) {
            for source in stack.sources().iter().filter(|source| source.layer == layer) {
                let Some(id) = source.node_id() else { continue };
                if selected.contains(&id) {
                    continue;
                }
                if let Some(index) =
                    sources.iter().position(|candidate| candidate.node_id() == Some(id))
                {
                    sources[index].weight = sources[index].weight.max(source.weight);
                    continue;
                }
                sources.push(*source);
                let shared = matches!(layer, Layer::Style | Layer::World)
                    || owners.get(&id).is_some_and(|values| values.len() > 1);
                scopes.push(if shared {
                    SceneScope::Shared
                } else {
                    SceneScope::Subject(*subject)
                });
            }
        }
    }

    for (subject, stack) in subjects.iter().zip(&stacks) {
        let source = stack
            .sources()
            .iter()
            .find(|source| source.node_id() == Some(*subject) && source.reached == Reached::Subject)
            .copied()
            .ok_or(SceneError::MissingSubject(*subject))?;
        sources.push(source);
        scopes.push(SceneScope::Subject(*subject));
    }
    sources.push(ResolvedSource {
        layer: Layer::Shot,
        origin: Origin::Shot(shot.label),
        reached: Reached::Shot,
        distance: 0,
        weight: shot.weight.clamp(0.0, 1.0),
    });
    scopes.push(SceneScope::Shared);

    Ok(ResolvedScene {
        subjects: subjects.to_vec(),
        stack: ResolvedStack { subject: subjects[0], sources },
        scopes,
    })
}

#[cfg(test)]
mod tests {
    use wobu_core::{Description, Link, LinkRole, Node, NodeKind, SectionValue};

    use super::*;

    #[test]
    fn shared_roots_and_culture_are_once_and_subjects_keep_caller_order() {
        let style = Node::new(NodeKind::StyleGuide, "House style").unwrap();
        let world_bible = Node::new(NodeKind::WorldBible, "World bible").unwrap();
        let culture = Node::new(NodeKind::Culture, "Cinder Guild").unwrap();
        let mut first = Node::new(NodeKind::Character, "Kael").unwrap();
        let mut second = Node::new(NodeKind::Character, "Mira").unwrap();
        first.links.push(Link::new(culture.id, LinkRole::MemberOf));
        second.links.push(Link::new(culture.id, LinkRole::MemberOf));
        first.description = Some(Description::from_sections([(
            "palette".into(),
            SectionValue::List(vec!["#111111".into()]),
        )]));
        second.description = Some(Description::from_sections([(
            "palette".into(),
            SectionValue::List(vec!["#eeeeee".into()]),
        )]));
        let world = World::new([&style, &world_bible, &culture, &first, &second]);

        let scene = resolve_scene(&world, &[second.id, first.id], Shot::new("Scene")).unwrap();
        let names: Vec<_> = scene.stack().sources().iter().map(ResolvedSource::name).collect();

        assert_eq!(names.iter().filter(|name| **name == "House style").count(), 1);
        assert_eq!(names.iter().filter(|name| **name == "Cinder Guild").count(), 1);
        assert_eq!(&names[names.len() - 3..], ["Mira", "Kael", "Scene"]);
        let culture_index = names.iter().position(|name| *name == "Cinder Guild").unwrap();
        assert_eq!(scene.scopes()[culture_index], SceneScope::Shared);

        let extracted = scene_fragments(&world, &scene);
        let shot_sections: Vec<_> = extracted
            .iter()
            .filter(|fragment| fragment.node_id().is_none())
            .map(|fragment| fragment.section())
            .collect();
        assert_eq!(shot_sections, ["framing", "scene_identity"]);
        let palettes: Vec<_> = extracted
            .iter()
            .filter(|fragment| fragment.section() == "palette")
            .filter_map(|fragment| fragment.node_id())
            .collect();
        assert_eq!(palettes, [second.id, first.id], "entity palettes remain separately scoped");
    }

    #[test]
    fn a_shared_source_uses_the_strongest_path_instead_of_summing_paths() {
        let culture = Node::new(NodeKind::Culture, "Guild").unwrap();
        let mut first = Node::new(NodeKind::Character, "First").unwrap();
        let mut second = Node::new(NodeKind::Character, "Second").unwrap();
        let mut low = Link::new(culture.id, LinkRole::MemberOf);
        low.weight = 0.25;
        let mut high = Link::new(culture.id, LinkRole::MemberOf);
        high.weight = 0.8;
        first.links.push(low);
        second.links.push(high);
        let world = World::new([&culture, &first, &second]);

        let scene = resolve_scene(&world, &[first.id, second.id], Shot::new("Scene")).unwrap();
        let guild = scene
            .stack()
            .sources()
            .iter()
            .find(|source| source.node_id() == Some(culture.id))
            .unwrap();

        assert_eq!(guild.weight, 0.8);
    }
}
