//! #209. The same sentence, pasted into two places, found by its digest.
//!
//! A [`Revision`] is a digest of wording plus [`Provenance`](crate::Provenance),
//! so two wordings that share one are not similar — they are the same words with
//! the same origin, written down twice. Nothing else in the project reports that,
//! and the failure it hides is quiet: nineteen quest summaries imported as copies
//! of the first dialogue line of their scene pass every other diagnostic, so the
//! writer sees a clean Text library while the game shows dialogue where an
//! objective should be.
//!
//! Two things this deliberately is not.
//!
//! **It is not a similarity check.** Two wordings that differ only in provenance
//! hash differently and are not reported, and that is the correct answer rather
//! than a limitation: the claim being made is "this is one wording stored twice",
//! which is exactly what one digest in two places means. Prose that merely reads
//! alike is a judgement nobody asked this module to make.
//!
//! **It is not a rule.** A repeated line is often deliberate — a catchphrase, a
//! sign read twice, a refrain — so every finding can be suppressed with a
//! [`WordingSuppression`], and the suppression is keyed to the revision rather
//! than to the sites. Keying it to the sites would mean moving a scene's beat
//! around silently revived a warning somebody had already answered; keying it to
//! the digest means the answer lasts exactly as long as the wording it was about.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::diagnose::Site;
use crate::id::{DialogueSlotId, Revision, SceneId, TextAssetId, VariantId};
use crate::scene::Scene;
use crate::text::TextAsset;

/// Which document a wording lives in.
///
/// Two variants rather than one id, because a caller turning a finding into a
/// selection has to open a different editor for each — the same distinction
/// [`Site::Scene`] and [`Site::TextAsset`] draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum WordingContainer {
    Scene(SceneId),
    TextAsset(TextAssetId),
}

impl WordingContainer {
    /// The document's id, for a caller that only needs to name the file.
    pub fn id(self) -> wobu_core::Id {
        match self {
            WordingContainer::Scene(id) => id.raw(),
            WordingContainer::TextAsset(id) => id.raw(),
        }
    }
}

/// One place a wording appears.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WordingSite {
    pub container: WordingContainer,
    /// A display name for the container, so a finding reads as prose without a
    /// second lookup per site.
    pub container_name: String,
    pub slot: DialogueSlotId,
    pub variant: VariantId,
    /// Keyed finely enough to select this copy and no other.
    pub site: Site,
}

/// One wording that is stored in more than one place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DuplicatedWording {
    pub revision: Revision,
    /// The shared words, so a reader can see what was duplicated without opening
    /// any of the sites.
    pub body: String,
    /// Every copy, in document then document order. At least two, by
    /// construction: a wording in one place is not duplicated.
    pub sites: Vec<WordingSite>,
}

impl DuplicatedWording {
    /// The sentence a diagnostic shows.
    pub fn message(&self) -> String {
        format!(
            "this wording is stored in {} places — {}. A revision is a digest of the words and \
             their provenance, so these are one line written down more than once. If the \
             repetition is deliberate, suppress it with a reason.",
            self.sites.len(),
            self.sites
                .iter()
                .map(|site| format!("{} ({})", site.container_name, site.site))
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}

/// A repetition somebody has looked at and signed off.
///
/// The rationale is not optional and not decoration: a suppression with no
/// reason is indistinguishable from a warning somebody dismissed to make a list
/// go green, and the next person to read it has no way to tell which it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WordingSuppression {
    pub revision: Revision,
    pub rationale: String,
}

impl WordingSuppression {
    /// Whether this suppression says anything. A blank rationale suppresses
    /// nothing, so a record that lost its reason stops hiding the finding rather
    /// than hiding it silently.
    pub fn is_stated(&self) -> bool {
        !self.rationale.trim().is_empty()
    }
}

/// Every wording stored in more than one place across the whole project.
///
/// Project-level because the question is: one scene cannot tell whether its line
/// also appears in a quest summary. Scenes and assets are handed in together for
/// that reason, and the result is ordered — by revision, then by site — so a list
/// rendered from it does not reshuffle between runs.
///
/// The *stored* revision is grouped on, not the recomputed one. A file whose
/// stored revision has drifted from its words is already
/// [`Problem::RevisionMismatch`](crate::Problem), and repairing it here would
/// make this check disagree with the locale pack, the recording script and every
/// approval — all of which are keyed to what the file says.
pub fn duplicated_wording(
    scenes: &[Scene],
    texts: &[TextAsset],
    suppressions: &[WordingSuppression],
) -> Vec<DuplicatedWording> {
    let mut by_revision: BTreeMap<Revision, (String, Vec<WordingSite>)> = BTreeMap::new();
    let mut record = |revision: &Revision, body: &str, site: WordingSite| {
        by_revision
            .entry(revision.clone())
            .or_insert_with(|| (body.to_owned(), Vec::new()))
            .1
            .push(site);
    };

    for scene in scenes {
        // A supporting-text authoring adapter is the same asset as its canonical
        // document, so counting both would report every line of it as duplicated
        // with itself.
        if scene.supporting_text.is_some() {
            continue;
        }
        for (beat, slot) in scene.dialogue_slots() {
            for variant in &slot.variants {
                record(
                    &variant.text.revision,
                    &variant.text.body,
                    WordingSite {
                        container: WordingContainer::Scene(scene.id),
                        container_name: scene.name.clone(),
                        slot: slot.id,
                        variant: variant.id,
                        site: Site::Variant { beat, slot: slot.id, variant: variant.id },
                    },
                );
            }
        }
    }
    for asset in texts {
        for entry in &asset.entries {
            for slot in &entry.lines {
                for variant in &slot.variants {
                    record(
                        &variant.text.revision,
                        &variant.text.body,
                        WordingSite {
                            container: WordingContainer::TextAsset(asset.id),
                            container_name: asset.name.clone(),
                            slot: slot.id,
                            variant: variant.id,
                            site: Site::TextVariant {
                                entry: entry.id,
                                slot: slot.id,
                                variant: variant.id,
                            },
                        },
                    );
                }
            }
        }
    }

    let suppressed: std::collections::BTreeSet<&Revision> = suppressions
        .iter()
        .filter(|suppression| suppression.is_stated())
        .map(|suppression| &suppression.revision)
        .collect();

    by_revision
        .into_iter()
        .filter(|(revision, (_, sites))| sites.len() > 1 && !suppressed.contains(revision))
        .map(|(revision, (body, mut sites))| {
            // One variant id cannot be two copies: a repeated identity is the
            // `duplicate_id` diagnostic, and counting it here would report a
            // malformed file as a writing problem.
            sites.sort_by_key(|site| (site.container, site.variant));
            sites.dedup_by_key(|site| site.variant);
            DuplicatedWording { revision, body, sites }
        })
        .filter(|found| found.sites.len() > 1)
        .collect()
}
