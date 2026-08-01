//! What the model is shown, and what it is told.
//!
//! Two strings and they are kept apart, because they answer different questions
//! and only one of them changes per node. [`SYSTEM`] is the standing
//! instructions — the four rules from `docs/04-influence-engine.md`, which "matter
//! more than the prompt wording" — and it goes in `EnhanceRequest::system`,
//! which both adapters put in the field their vendor documents for it
//! (Anthropic's top-level `system`, Gemini's `system_instruction`). Folding it
//! into the user turn measurably weakens it, which is the whole reason
//! `provider.rs` carries the two separately.
//!
//! The instructions live in `system.md` as prose rather than in `format!` calls
//! here, because they are the part of this feature most likely to be revised by
//! someone reading the output and least likely to be revised well through string
//! concatenation. Nothing is interpolated into them.
//!
//! [`build`] is the other half: the material. Its shape is
//! `docs/04-influence-engine.md`'s step 1, and each clause of that step is
//! load-bearing —
//!
//! - **the resolved stack's descriptions, not their raw notes.** A layer's notes
//!   are one person's shorthand; its description is what actually reaches a
//!   renderer, and rule 3 ("do not restate inherited traits") is only checkable
//!   against the thing that will be in the compiled prompt beside this one.
//! - **this node's `notes_raw`.** Verbatim, unedited. It is the only input that
//!   is allowed to introduce a fact.
//! - **its attributes.** A height in centimetres changes what is drawn.
//! - **the names of its reference images' roles.** Not the images: a role that
//!   is already pinned by a picture does not need pinning again in words, and
//!   that is a thing the model can only know if it is told.

use serde_json::Value;
use wobu_core::{Id, Node, SectionValue, kind_def};
use wobu_influence::{World, resolve};

/// The standing instructions, as prose a person can edit.
pub const SYSTEM: &str = include_str!("system.md");

/// One Enhance's worth of material, and the walk it came from.
pub struct Context<'a> {
    pub subject: &'a Node,
    /// The user turn.
    pub prompt: String,
    /// Every source the stack resolved to, in stack order, including the
    /// subject. Passed to `Project::accept_enhanced` untouched — it drops the
    /// subject itself and stamps the rest, so that "is this description still
    /// current" is answered against the same walk that builds the prompt.
    pub sources: Vec<Id>,
}

/// Build the context for one subject, or `None` when it is not in this world.
///
/// Pure, and over borrowed nodes rather than a project handle, for the same
/// reason `wobu-influence` is: this runs under the project mutex and must not do
/// IO beneath it.
pub fn build(nodes: &[Node], subject_id: Id) -> Option<Context<'_>> {
    let world = World::new(nodes.iter());
    // No `Shot`. Layer 7 is framing for one picture — a preset's aspect and
    // camera — and this is not a picture, it is the canon every future picture
    // of this entity will be compiled from. Framing text in here would be a
    // model told that "three-quarter view, flat light" is a fact about the
    // entity.
    let stack = resolve(&world, subject_id, None)?;
    let subject = stack
        .subject_source()
        .and_then(|source| source.node())
        // `subject_source` is `None` when the subject was reached as an outer
        // layer first — a project whose Style Guide is the selected node. It is
        // still in the world, because `resolve` answered at all.
        .or_else(|| nodes.iter().find(|n| n.id == subject_id))?;

    let mut prompt = format!(
        "Describe **{}**, a {}, as canon for this world.\n",
        subject.name,
        kind_def(subject.kind).label,
    );

    prompt.push_str("\n# Inherited context\n\n");
    prompt.push_str(
        "These layers are already established and are compiled into the same prompt as \
         what you write. Build on them; do not repeat them.\n",
    );
    let mut inherited = 0;
    for source in stack.sources() {
        let Some(node) = source.node() else { continue };
        // The subject's own card is in its own stack. Its description is the
        // *output* of this call, and showing the previous one back turns a
        // re-enhance into a paraphrase of itself — which is exactly how a
        // description stops tracking the notes it is supposed to be a reading of.
        if node.id == subject.id {
            continue;
        }
        let block = described(node);
        if !block.is_empty() {
            inherited += 1;
            prompt.push_str(&block);
        }
    }
    if inherited == 0 {
        // Said out loud rather than left as an absence. A model shown no
        // upstream layers cannot tell "nothing has been described yet" from
        // "you were not sent it", and only one of those means rule 3 has
        // nothing to bite on.
        prompt.push_str("\nNothing upstream of this entity has been described yet.\n");
    }

    prompt.push_str("\n# The subject\n");
    if !subject.summary.trim().is_empty() {
        prompt.push_str(&format!("\nOne line: {}\n", subject.summary.trim()));
    }

    prompt.push_str("\n## Notes, verbatim\n\n");
    let notes = subject.notes_raw.trim();
    if notes.is_empty() {
        // Not a refusal. A node that inherits a full stack can legitimately be
        // described from its name and its attributes, and the honest answer to
        // a thin one is a short description with questions beside it — which is
        // rule 1, and is why this is a sentence rather than an error.
        prompt.push_str("Nothing has been written yet.\n");
    } else {
        prompt.push_str(notes);
        prompt.push('\n');
    }

    if let Some(block) = attributes(subject) {
        prompt.push_str(&block);
    }
    if let Some(block) = reference_roles(subject) {
        prompt.push_str(&block);
    }

    Some(Context {
        subject,
        prompt,
        sources: stack.sources().iter().filter_map(|s| s.node_id()).collect(),
    })
}

/// One upstream layer, as its *description* — never its notes.
///
/// Empty when the node has nothing described, so that a layer nobody has written
/// yet contributes no heading rather than an empty one. Sections are walked in
/// the kind's declared order and labelled with the registry's own labels, so
/// what the model reads under "Costume" is what the editor renders under
/// "Costume".
fn described(node: &Node) -> String {
    let Some(description) = node.description.as_ref() else { return String::new() };
    let def = kind_def(node.kind);

    let mut rows = String::new();
    for section in def.sections {
        let Some(value) = description.sections.get(section.key) else { continue };
        if value.is_empty() {
            continue;
        }
        let text = match value {
            SectionValue::Text(text) => text.trim().to_owned(),
            // Semicolons rather than newlines: these are short phrases, and a
            // bullet each would make an inherited list look longer, and so read
            // as more important, than the prose beside it.
            SectionValue::List(items) => items
                .iter()
                .map(|i| i.trim())
                .filter(|i| !i.is_empty())
                .collect::<Vec<_>>()
                .join("; "),
        };
        rows.push_str(&format!("- {}: {}\n", section.label, text));
    }
    if rows.is_empty() {
        return rows;
    }
    format!("\n## {}: {}\n\n{rows}", def.label, node.name)
}

fn attributes(node: &Node) -> Option<String> {
    if node.attributes.is_empty() {
        return None;
    }
    let mut block = String::from("\n## Attributes\n\n");
    for (key, value) in &node.attributes {
        // A JSON string is unwrapped so `"ash-grey"` does not reach the model
        // wearing its quotes; everything else is printed as it is written,
        // because a number, a list or a nested object is already legible and
        // inventing a rendering for each would be inventing a schema.
        let value = match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        block.push_str(&format!("- {key}: {value}\n"));
    }
    Some(block)
}

/// The roles this entity's reference images are attached in — the names, and
/// nothing else.
///
/// Filtered on `is_conditioning`, which is `wobu-core`'s single answer to "can
/// this role ever leave the machine". `mood` is the one that cannot: it is the
/// board the artist keeps for themselves, and its *existence* is as much theirs
/// as its contents. Deriving the filter from the same function the compiler uses
/// is what stops the two lists disagreeing later, and the direction that
/// disagreement fails in is somebody's mood board being described to a vendor.
///
/// Disabled links are left out too. A muted reference does not reach a backend,
/// so telling the model that aspect is already pinned would be false.
fn reference_roles(node: &Node) -> Option<String> {
    let mut roles: Vec<&'static str> = node
        .asset_links
        .iter()
        .filter(|link| link.enabled && link.role.is_conditioning())
        .map(|link| link.role.label())
        .collect();
    roles.sort_unstable();
    roles.dedup();
    if roles.is_empty() {
        return None;
    }
    Some(format!(
        "\n## Reference images already attached\n\n{}\n",
        roles.join(", "),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wobu_core::asset::AssetRef;
    use wobu_core::{AssetRole, Description, Link, LinkRole, NodeKind};

    /// A style guide, a species, a culture and a character in all of them — the
    /// smallest world where "do not restate inherited traits" means anything.
    ///
    /// Every upstream node carries *both* notes and a description, and the two
    /// say different things on purpose: that is the only way a test can tell
    /// which of the two was sent.
    struct Ashfall {
        nodes: Vec<Node>,
        kael: Id,
    }

    fn described_node(kind: NodeKind, name: &str, sections: &[(&str, SectionValue)]) -> Node {
        let mut node = Node::new(kind, name).unwrap();
        node.notes_raw = format!("scratch notes for {name} that nothing should send");
        node.description = Some(Description::from_sections(
            sections.iter().map(|(k, v)| ((*k).to_string(), v.clone())),
        ));
        node
    }

    fn ashfall() -> Ashfall {
        let style = described_node(NodeKind::StyleGuide, "Ashfall House Style", &[(
            "rendering",
            SectionValue::Text("Ash-dusted, matte, hand-painted".into()),
        )]);
        let vashk = described_node(NodeKind::Species, "Vashk", &[
            ("anatomy", SectionValue::Text("Four-jointed digitigrade legs".into())),
            ("never", SectionValue::List(vec!["upright plantigrade stance".into()])),
        ]);
        let guild = described_node(NodeKind::Culture, "Cinder Guild", &[(
            "costume",
            SectionValue::Text("Ash-grey longcoats, brass fastenings".into()),
        )]);

        let mut kael = Node::new(NodeKind::Character, "Kael Vantris").unwrap();
        kael.summary = "Ashfall scout, guild-sworn.".into();
        kael.notes_raw = "Forelimb a joint short. Carries a bone censer.".into();
        kael.links = vec![
            Link::new(vashk.id, LinkRole::SpeciesOf),
            Link::new(guild.id, LinkRole::MemberOf),
        ];
        kael.attributes.insert("height_cm".into(), Value::from(190));
        kael.attributes.insert("hide".into(), Value::from("ash-grey"));
        kael.description = Some(Description::from_sections([(
            "silhouette".to_string(),
            SectionValue::Text("STALE PREVIOUS ANSWER".into()),
        )]));
        kael.asset_links = vec![
            AssetRef::new(wobu_core::new_id(), AssetRole::Pose),
            AssetRef::new(wobu_core::new_id(), AssetRole::Mood),
        ];

        let (kael_id, nodes) = (kael.id, vec![style, vashk, guild, kael]);
        Ashfall { nodes, kael: kael_id }
    }

    fn prompt(world: &Ashfall) -> String {
        build(&world.nodes, world.kael).expect("the subject is in this world").prompt
    }

    #[test]
    fn the_stack_arrives_as_descriptions_and_never_as_the_notes_behind_them() {
        // The first clause of step 1, and the one an implementation is most
        // likely to get wrong by reaching for `notes_raw` because it is the
        // field with the words in it. An upstream node's notes are one person's
        // shorthand; its description is what will be in the compiled prompt
        // beside this one, and rule 3 is only checkable against that.
        let text = prompt(&ashfall());

        assert!(text.contains("Four-jointed digitigrade legs"), "{text}");
        assert!(text.contains("Ash-grey longcoats, brass fastenings"), "{text}");
        assert!(text.contains("Ash-dusted, matte, hand-painted"), "{text}");
        assert!(!text.contains("scratch notes"), "an upstream node's notes were sent: {text}");

        // Attributed, in stack order, and with the registry's own labels — the
        // model has to be able to tell which layer established what, or it
        // cannot tell a deviation from a repetition.
        let style = text.find("Art Style: Ashfall House Style").expect(&text);
        let species = text.find("Species: Vashk").expect(&text);
        let culture = text.find("Culture: Cinder Guild").expect(&text);
        assert!(style < species && species < culture, "the stack order was lost: {text}");
        assert!(text.contains("- Anatomy: Four-jointed"), "{text}");
        assert!(text.contains("- Never: upright plantigrade stance"), "{text}");
    }

    #[test]
    fn the_subjects_notes_go_in_verbatim_and_its_previous_description_does_not() {
        // Both halves matter. The notes are the only input allowed to introduce
        // a fact, so they are not summarised or reformatted on the way in — and
        // the previous answer is left out, because a re-enhance shown its own
        // last answer paraphrases it instead of re-reading the notes, and the
        // description quietly stops tracking what it is supposed to describe.
        let text = prompt(&ashfall());

        assert!(text.contains("Forelimb a joint short. Carries a bone censer."), "{text}");
        assert!(!text.contains("STALE PREVIOUS ANSWER"), "the last answer was fed back: {text}");
        assert!(text.contains("Ashfall scout, guild-sworn."), "the summary is context too");
        assert!(text.starts_with("Describe **Kael Vantris**, a Character"), "{text}");
    }

    #[test]
    fn the_attributes_go_because_a_height_changes_what_is_drawn() {
        let text = prompt(&ashfall());
        assert!(text.contains("- height_cm: 190"), "{text}");
        // Unwrapped, so a string attribute does not arrive wearing its quotes.
        assert!(text.contains("- hide: ash-grey"), "{text}");
    }

    #[test]
    fn a_reference_images_role_is_named_and_the_mood_board_is_not() {
        // The point of sending roles at all: a pose reference exists, so the
        // model should not spend the description specifying a pose — and it can
        // only know that if it is told. The mood board is the exception, and it
        // is the whole exception: `mood` never leaves this machine, and that
        // covers the fact that one exists as much as it covers the picture.
        let text = prompt(&ashfall());

        assert!(text.contains("## Reference images already attached"), "{text}");
        assert!(text.contains("Pose"), "{text}");
        assert!(!text.to_lowercase().contains("mood"), "a mood board was disclosed: {text}");

        // And no id, no path, no filename — the roles and nothing else.
        let world = ashfall();
        for link in &world.nodes.last().unwrap().asset_links {
            assert!(
                !text.contains(&link.asset_id.to_string()),
                "an asset id crossed into the prompt: {text}",
            );
        }
    }

    #[test]
    fn a_disabled_reference_is_not_reported_as_pinning_anything() {
        // A muted link contributes nothing to a generation, so telling the model
        // that aspect is already fixed by a picture would be false — and the
        // sentence it would produce is a description that leaves out the one
        // thing nothing else is going to say.
        let mut world = ashfall();
        for link in &mut world.nodes.last_mut().unwrap().asset_links {
            link.enabled = false;
        }
        assert!(!prompt(&world).contains("Reference images already attached"));
    }

    #[test]
    fn a_subject_with_nothing_upstream_is_told_so_rather_than_left_to_assume() {
        // Every project between `project_create` and somebody writing a Style
        // Guide is in this state. A model that sees no layers cannot tell
        // "nothing has been described" from "you were not sent it", and only
        // one of those means there is nothing to avoid restating.
        let lonely = Node::new(NodeKind::Prop, "Ash Lantern").unwrap();
        let (id, nodes) = (lonely.id, vec![lonely]);

        let context = build(&nodes, id).expect("a lone node still has a context");
        assert!(context.prompt.contains("Nothing upstream"), "{}", context.prompt);
        assert!(context.prompt.contains("Nothing has been written yet."), "{}", context.prompt);
        assert_eq!(context.sources, vec![id], "the subject is its own stack");
    }

    #[test]
    fn a_subject_nobody_has_heard_of_has_no_context_rather_than_a_panic() {
        assert!(build(&ashfall().nodes, wobu_core::new_id()).is_none());
        assert!(build(&[], wobu_core::new_id()).is_none());
    }

    #[test]
    fn the_sources_are_the_walk_the_stamp_will_be_made_of() {
        // `accept_enhanced` stamps these and drops the subject itself. Handing
        // it the resolve's own answer rather than a list assembled here is what
        // keeps staleness and prompt compilation talking about one graph.
        let world = ashfall();
        let context = build(&world.nodes, world.kael).unwrap();
        for node in &world.nodes {
            assert!(context.sources.contains(&node.id), "{} is missing from the stamp", node.name);
        }
    }

    #[test]
    fn the_four_constraints_are_standing_instructions_rather_than_prompt_wording() {
        // `docs/04-influence-engine.md`: "the constraints given to the model
        // matter more than the prompt wording". They are in `system.md` and so
        // reach the field each vendor documents for standing instructions,
        // which is measurably stronger than the user turn — and they are the
        // same four for every node, which the per-node prompt is not.
        for claim in [
            "Do not invent facts the notes do not imply",
            "Write visually",
            "Do not restate anything the inherited layers already establish",
            "Populate `never`",
            // The mechanism rule 1 depends on. Without somewhere to put the
            // question, "ask rather than invent" is an instruction with no way
            // to be obeyed.
            "`questions`",
        ] {
            assert!(SYSTEM.contains(claim), "the system prompt no longer says: {claim}");
        }

        // And nothing per-node leaks into them: they are a constant, so an
        // interpolation here would be one node's name shown to every other.
        assert!(!SYSTEM.contains("{"), "the standing instructions interpolate something");
        assert!(!prompt(&ashfall()).contains("Do not invent facts"), "said twice");
    }
}
