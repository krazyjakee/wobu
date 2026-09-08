//! Checks shared by Script, Flow and Source writes. Drafts may be incomplete,
//! but an approval cannot silently follow different wording.

use std::collections::BTreeMap;

use wobu_narrative::{ReviewState, Scene};

use crate::error::{Code, CommandResult, WobuError};

pub(super) fn validate_approval(
    previous: Option<&Scene>,
    next: &Scene,
    restoring: bool,
) -> CommandResult<()> {
    let previous = previous
        .into_iter()
        .flat_map(Scene::dialogue_slots)
        .flat_map(|(_, slot)| slot.variants.iter().map(move |v| ((slot.id, v.id), &v.text)))
        .collect::<BTreeMap<_, _>>();
    for (_, slot) in next.dialogue_slots() {
        for variant in &slot.variants {
            let text = &variant.text;
            if text.lifecycle.review != ReviewState::Approved {
                continue;
            }
            if !text.revision_matches() {
                return Err(WobuError::new(
                    Code::Invalid,
                    format!(
                        "Dialogue variant {} has an approval for different wording. Set its \
                         review to draft and update its revision before approving it again.",
                        variant.id
                    ),
                ));
            }
            if !restoring
                && let Some(old) = previous.get(&(slot.id, variant.id))
                && (old.body != text.body || old.provenance != text.provenance)
            {
                return Err(WobuError::new(
                    Code::Invalid,
                    format!(
                        "Dialogue variant {} changed. Save it as a draft before approving \
                         the new wording.",
                        variant.id
                    ),
                ));
            }
        }
    }
    Ok(())
}
