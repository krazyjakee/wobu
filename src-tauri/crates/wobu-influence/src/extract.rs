//! Turning the resolved stack into fragments.

use wobu_core::{FragmentTarget, Node, Preset, SectionValue, kind_def};

use crate::fragment::{Fragment, FragmentBody, Sliders, section_target};
use crate::stack::{Origin, ResolvedSource, ResolvedStack};

/// The key the Shot layer's framing text is filed under.
///
/// Not a description section, and no kind declares it, because no node owns it:
/// it is the preset's own sentence about pose, light and distance
/// (`Preset::framing`). `Preset::section_priority` therefore answers 1.0 for it
/// under every preset, which is the right answer — a preset must not be able to
/// reweight its own framing relative to another preset's.
const FRAMING: &str = "framing";

/// Extract everything a resolved stack contributes.
///
/// The three terms of `link.weight × section_priority × user_slider`
/// (`docs/04-influence-engine.md`) come from three different places and are kept
/// that way: the path product is [`ResolvedSource::weight`], produced by
/// resolution; the section multiplier is the output preset's
/// (`Preset::section_priority`), which is the registry's business and not
/// re-derived here; and the slider is the caller's per-generation [`Sliders`].
/// A preset is required rather than optional because every kind has a default
/// one, so there is no state in which the app does not know which sheet it is
/// weighting for, and an "unweighted" mode would be a fourth answer nobody asked
/// for.
///
/// Fragments come out in stack order — layer by layer, source by source, and
/// within a source in the order its kind declares its sections — never in weight
/// order. That is the order the layer cards read in and the order the prompt is
/// joined in, subject last where the recency bias helps (see the crate docs).
/// Sorting by weight is the compiler's business and it needs the reading order
/// to have survived to do it.
///
/// Nothing is filtered out here. A fragment weighted to zero and a
/// `moodboard_only` asset that may never be sent are both in the result: the
/// first because the user turned it down rather than deleted it, the second
/// because the human is meant to see it on the moodboard. Anything assembling a
/// request filters on [`Fragment::is_sendable`], and the budget (#43) takes this
/// slice and records what it drops rather than removing it.
pub fn fragments<'a>(
    stack: &ResolvedStack<'a>,
    preset: &Preset,
    sliders: &Sliders,
) -> Vec<Fragment<'a>> {
    let mut out: Vec<Fragment<'a>> = Vec::new();
    for source in stack.sources() {
        let slider = sliders.for_source(source);
        match source.origin {
            Origin::Node(node) => {
                text_fragments(&mut out, source, node, preset, slider);
                asset_fragments(&mut out, source, node, preset, slider);
            }
            Origin::Shot(_) => framing_fragment(&mut out, source, preset, slider),
        }
    }
    out
}

/// A node's structured description: one fragment per prose section, one per item
/// of a list section.
///
/// Per item rather than per list because the budget drops fragments, and a
/// `never` list that had to go all at once would take four good negatives out to
/// shed one weak one.
///
/// Read through the kind's declared sections rather than by iterating the
/// description's own map. That is the order the editor renders, and the order
/// the store already normalises files into on read (`Description::normalised_for`
/// in `wobu-store`'s markdown reader), so a hand-edited file with its sections
/// shuffled compiles to the same prompt as one the app wrote. A section the kind
/// does not declare is one nothing knows how to weight, and it is dropped here
/// for the same reason it is dropped there.
///
/// `notes_raw` is deliberately not a source. The pipeline is notes → description
/// → prompt (`docs/04-influence-engine.md`); compiling the raw notes as well
/// would put the user's unedited jottings into the prompt beside the description
/// written from them and state every fact twice, which is the duplication the
/// whole layering discipline exists to avoid.
fn text_fragments<'a>(
    out: &mut Vec<Fragment<'a>>,
    source: &ResolvedSource<'a>,
    node: &'a Node,
    preset: &Preset,
    slider: f32,
) {
    let Some(description) = node.description.as_ref() else { return };
    for def in kind_def(node.kind).sections {
        let Some(value) = description.sections.get(def.key) else { continue };
        // A section the kind declares that the preset says nothing about is
        // ordinary: the registry's documented answer for that is 1.0, and a
        // preset that boosts a section this kind never declares simply never
        // meets one. Neither is an error and neither is worth reporting.
        let weight = source.weight * preset.section_priority(def.key) * slider;
        let target = section_target(def.key);
        match value {
            SectionValue::Text(text) => push_text(out, source, def.key, text, weight, target),
            // Matched on the value rather than on `def.value_kind` so that a
            // hand-edited file which put a list where the kind declares prose
            // still contributes what it has, instead of silently contributing
            // nothing until someone opens the editor.
            SectionValue::List(items) => {
                for item in items {
                    push_text(out, source, def.key, item, weight, target);
                }
            }
        }
    }
}

/// One text fragment, unless it would be blank.
///
/// A half-filled description is the normal state between an enhance finishing
/// and the user editing it, and it is blank per section rather than per node. An
/// empty fragment would be a row on the layer card with nothing in it, an empty
/// span in the compiled prompt, and budget spent on nothing.
fn push_text<'a>(
    out: &mut Vec<Fragment<'a>>,
    source: &ResolvedSource<'a>,
    section: &'static str,
    text: &'a str,
    weight: f32,
    target: FragmentTarget,
) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    out.push(Fragment::new(source, section, FragmentBody::Text(text), weight, target));
}

/// A node's reference images, one fragment each, routed by role.
///
/// The role decides the target and `wobu-core` decides what a role targets
/// (`AssetRole::target`). That is what keeps a `mood` reference off a backend,
/// and the reason this calls it rather than mapping roles itself: there is one
/// mapping in the workspace and this must not become a second. A mood reference
/// still becomes a fragment — it is on the moodboard, and the layer card counts
/// it — and [`Fragment::is_sendable`] is what holds it back.
///
/// `enabled` is the world's own off switch, exactly as it is for influence
/// links, so a disabled reference contributes nothing at all. Muting for one
/// generation is the Inspector's job and never reaches this crate.
///
/// The reference's own weight multiplies into the path product rather than
/// standing beside it: it is the last edge of the path from the subject to the
/// picture, so a reference attached at 0.5 to a culture the subject is held to at
/// 0.5 is a quarter of the way in. Same rule as `resolve`, and clamped for the
/// same reason — weights come off disk where anyone can type 4.0.
///
/// A role whose name is also a section key picks up that section's priority,
/// which is what you want: a costume plate should lean on the costume references
/// as well as on the costume prose. A role whose name is not one — `full_ref`,
/// and `material` against the `materials` section — sits at the 1.0 the registry
/// documents for a section a preset has no opinion about.
fn asset_fragments<'a>(
    out: &mut Vec<Fragment<'a>>,
    source: &ResolvedSource<'a>,
    node: &'a Node,
    preset: &Preset,
    slider: f32,
) {
    for reference in &node.asset_links {
        if !reference.enabled {
            continue;
        }
        let section = reference.role.as_str();
        let path = source.weight * reference.weight.clamp(0.0, 1.0);
        out.push(Fragment::new(
            source,
            section,
            FragmentBody::Asset { id: reference.asset_id, role: reference.role },
            path * preset.section_priority(section) * slider,
            reference.role.target(),
        ));
    }
}

/// The Shot layer's contribution: the preset's framing text.
///
/// Layer 7 has no node, so its fragment comes from the preset itself. Framing is
/// written as prose "because it is compiled into the prompt alongside every
/// other fragment rather than being a parameter the backend understands"
/// (`wobu-core`'s `preset.rs`), and a fragment is the only form in which it can
/// arrive carrying a layer — without one, the pose and lighting instructions
/// would be the single part of the prompt the Inspector could not attribute.
///
/// A stack resolved without a shot — `influence_resolve`, before any generation
/// is set up — has no Shot source and so gets no framing text, which is right:
/// nothing has been framed yet.
fn framing_fragment<'a>(
    out: &mut Vec<Fragment<'a>>,
    source: &ResolvedSource<'a>,
    preset: &Preset,
    slider: f32,
) {
    let weight = source.weight * preset.section_priority(FRAMING) * slider;
    push_text(out, source, FRAMING, preset.framing, weight, FragmentTarget::Prompt);
}
