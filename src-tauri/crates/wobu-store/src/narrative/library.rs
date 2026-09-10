//! Bounded discovery records derived from canonical scenes, never writable source.
use serde::{Deserialize, Serialize};
use wobu_narrative::{Freshness, GenerationPolicy, ReviewState, Scene};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub generated: usize,
    pub edited: usize,
    pub locked: usize,
    pub needs_review: usize,
    pub out_of_date: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub rel: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Projection {
    pub summary: Summary,
    pub act_id: Option<String>,
    pub arc_id: Option<String>,
    pub tag_ids: Vec<String>,
    /// The scene's setting node (#206). Always written, `null` included, because
    /// the index reads the key's presence to decide whether a stored projection
    /// predates this field and has to be rebuilt — a skipped key and an absent
    /// setting would be indistinguishable.
    #[serde(default)]
    pub setting_id: Option<String>,
    pub participants: Vec<String>,
    pub slots: usize,
    pub filled: usize,
    pub beats: usize,
    pub counts: Counts,
    /// Rebuildable arc-only metadata. None marks a projection from an older app.
    #[serde(default)]
    pub arc: Option<ArcDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArcExit {
    pub beat_id: String,
    pub route_id: String,
    pub choice: bool,
    pub label: String,
    pub to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArcDetails {
    pub exits: Vec<ArcExit>,
    pub needs_text: usize,
}

pub fn arc_details(scene: &Scene) -> ArcDetails {
    use wobu_narrative::Destination;
    let mut exits = Vec::new();
    let mut needs_text = 0;
    for beat in &scene.beats {
        needs_text += usize::from(beat.dialogue.is_empty());
        needs_text += beat
            .dialogue
            .iter()
            .filter(|slot| {
                slot.variants.is_empty()
                    || slot.variants.iter().any(|v| v.text.body.trim().is_empty())
            })
            .count();
        for (route_id, choice, label, to) in beat
            .choices
            .iter()
            .map(|route| (route.id.to_string(), true, route.label.clone(), &route.to))
            .chain(
                beat.outcomes
                    .iter()
                    .map(|route| (route.id.to_string(), false, "Outcome".into(), &route.to)),
            )
        {
            let to = match to {
                Destination::Scene(id) => Some(id.to_string()),
                Destination::Unresolved {} => None,
                _ => continue,
            };
            exits.push(ArcExit { beat_id: beat.id.to_string(), route_id, choice, label, to });
        }
    }
    ArcDetails { exits, needs_text }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub scene_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_id: Option<String>,
    pub snippet: String,
    pub draft: bool,
}
pub(crate) struct TextRow {
    pub target: Match,
    pub text: String,
}
pub(crate) struct VariantRow {
    pub policy: &'static str,
    pub review: &'static str,
    pub freshness: &'static str,
}
pub(crate) fn project(scene: &Scene, rel: &str) -> (Projection, Vec<TextRow>, Vec<VariantRow>) {
    let mut result = Projection {
        summary: Summary {
            id: scene.id.to_string(),
            name: scene.name.clone(),
            slug: rel.rsplit('/').next().unwrap_or(rel).trim_end_matches(".yaml").into(),
            rel: rel.into(),
        },
        act_id: scene.act_id.map(|id| id.to_string()),
        arc_id: scene.arc_id.map(|id| id.to_string()),
        tag_ids: scene.tag_ids.iter().map(ToString::to_string).collect(),
        setting_id: scene.setting_id.map(|id| id.to_string()),
        participants: scene.participants.iter().map(|p| p.entity.to_string()).collect(),
        slots: 0,
        filled: 0,
        beats: scene.beats.len(),
        counts: Counts::default(),
        arc: Some(arc_details(scene)),
    };
    let mut texts = Vec::new();
    let mut variants = Vec::new();
    let mut add = |text: &str,
                   beat_id: Option<String>,
                   line_id: Option<String>,
                   variant_id: Option<String>,
                   draft| {
        if !text.is_empty() {
            texts.push(TextRow {
                target: Match {
                    scene_id: scene.id.to_string(),
                    beat_id,
                    line_id,
                    variant_id,
                    snippet: String::new(),
                    draft,
                },
                text: text.into(),
            });
        }
    };
    add(&scene.name, None, None, None, false);
    add(&scene.summary, None, None, None, false);
    for beat in &scene.beats {
        let beat_id = Some(beat.id.to_string());
        add(&beat.title, beat_id.clone(), None, None, false);
        for text in beat.intents.iter().map(|i| &i.intent).chain(&beat.must_convey) {
            add(text, beat_id.clone(), None, None, false);
        }
        for slot in &beat.dialogue {
            result.slots += 1;
            result.filled +=
                usize::from(slot.variants.iter().any(|v| !v.text.body.trim().is_empty()));
            for variant in &slot.variants {
                let lifecycle = &variant.text.lifecycle;
                let policy = if slot.policy == GenerationPolicy::Locked {
                    GenerationPolicy::Locked
                } else {
                    lifecycle.policy
                };
                let policy = match policy {
                    GenerationPolicy::Generated => {
                        result.counts.generated += 1;
                        "generated"
                    }
                    GenerationPolicy::Edited => {
                        result.counts.edited += 1;
                        "edited"
                    }
                    GenerationPolicy::Locked => {
                        result.counts.locked += 1;
                        "locked"
                    }
                };
                let review = match lifecycle.review {
                    ReviewState::Draft => "draft",
                    ReviewState::Approved => "approved",
                };
                let freshness = match lifecycle.freshness {
                    Freshness::Current => "current",
                    Freshness::OutOfDate => "out_of_date",
                };
                result.counts.needs_review +=
                    usize::from(lifecycle.review != ReviewState::Approved);
                result.counts.out_of_date +=
                    usize::from(lifecycle.freshness == Freshness::OutOfDate);
                variants.push(VariantRow { policy, review, freshness });
                let draft =
                    matches!(variant.text.provenance, wobu_narrative::Provenance::Generated { .. })
                        && lifecycle.review != ReviewState::Approved;
                add(
                    &variant.text.body,
                    beat_id.clone(),
                    Some(slot.id.to_string()),
                    Some(variant.id.to_string()),
                    draft,
                );
            }
        }
    }
    (result, texts, variants)
}
