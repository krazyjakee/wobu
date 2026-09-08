//! Supporting text: barks, ambient exchanges, companion reactions, codex
//! entries, quest summaries and journals (#167).
//!
//! Everything a game says outside a scene. A guard muttering as the player
//! walks past, two dockhands finishing an argument, a companion reacting to a
//! looted relic, the codex page that explains what the relic is, the quest log
//! line that says why it matters, and the journal entry the player writes
//! afterwards. Six kinds, one model, because they differ in exactly two ways —
//! who may voice them and how many lines one delivery is — and inventing six
//! record types to express two rules would guarantee that a fix to the
//! freshness rule landed in four of them.
//!
//! ## What this is not, and must never become
//!
//! **It is not a second, unreviewed way to write text.** A supporting line is a
//! [`DialogueSlot`] holding [`Variant`](crate::Variant)s of [`Text`](crate::Text),
//! exactly as a beat's lines are. That is not a convenience: it is what makes a
//! bark carry the same stable slot and variant identities, the same
//! [`Revision`](crate::Revision), the same [`Provenance`](crate::Provenance) and
//! the same [`ContentLifecycle`](crate::ContentLifecycle) as scene dialogue, so
//! the review queue (#166), the generation policy (#165), the freshness
//! recomputation (#168), the locale pack and the recording script (#178, #179)
//! all see one kind of thing. If
//! this module ever grows its own `body: String` beside a slot, every one of
//! those stops being true for a quarter of the game's words.
//!
//! **It is not a graph.** There is no [`Choice`](crate::Choice), no
//! [`Outcome`](crate::Outcome) and no [`Destination`](crate::Destination) in
//! this file, and there is no field where one could be put. US-09 requires that
//! standalone prose exists without inventing a player choice to hang it on; the
//! way to guarantee that is for the type to have nowhere to put a fake one,
//! rather than for a validator to notice one and complain. A codex entry that
//! could branch the story would also have to be reachability-analysed, covered
//! by scenarios and drawn on the Flow canvas, and none of those things are true
//! of a codex entry.
//!
//! **It does not decide when it plays.** [`Trigger`] records the host-side
//! moment an asset belongs to and the state condition that has to hold; picking
//! one entry out of the eligible ones, and remembering that it played, is the
//! runtime's job (#159) under the policy in [`RepeatPolicy`]. This module only
//! writes the policy down.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::diagnose::{Diagnostic, Problem, Site};
use crate::expr::Condition;
use crate::id::{DialogueSlotId, SceneId, TextAssetId, TextEntryId, VariantId};
use crate::lifecycle::GenerationPolicy;
use crate::scene::{DialogueSlot, EntityId, Participant, Speaker, Tombstone, TombstoneTarget};
use crate::state::{Name, StateSchema};

/// The narrative source format for supporting text documents.
///
/// Its own number rather than a share of `SCENE_SCHEMA_VERSION`, because the two
/// formats change for unrelated reasons: adding a field to a bark must not make
/// every scene in the project look like it needs migrating, and a project that
/// contains no supporting text at all must stay readable by a build that has
/// never heard of it.
pub const TEXT_SCHEMA_VERSION: u32 = 1;

/// Which of the six supporting kinds an asset is.
///
/// A closed enum rather than a free-text template name, because two behaviours
/// are derived from it — who may speak ([`TextKind::voice`]) and how many lines
/// one delivery contains ([`TextKind::delivery`]) — and a spelling mistake in a
/// template name would silently produce an asset that obeys neither rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextKind {
    /// One short line thrown at the player in passing: a guard's challenge, a
    /// merchant's greeting, a companion's complaint about the weather.
    Bark,
    /// A short exchange between two or more characters that the player
    /// overhears rather than joins. The only kind whose delivery is a sequence
    /// of lines with different speakers.
    Ambient,
    /// A companion's response to something that just happened.
    Reaction,
    /// An encyclopaedia page. Prose, read in a menu, with no speaker.
    Codex,
    /// The one or two sentences a quest log shows about where the player is up
    /// to. Prose.
    QuestSummary,
    /// A first-person record of what happened, written in the player's voice.
    /// Prose, and the reason [`Voice`] admits [`Speaker::Player`] as well as
    /// [`Speaker::Narrator`].
    Journal,
}

impl TextKind {
    /// Every kind, in the order a template picker should offer them: the spoken
    /// three first, then the written three.
    pub const ALL: [TextKind; 6] = [
        TextKind::Bark,
        TextKind::Ambient,
        TextKind::Reaction,
        TextKind::Codex,
        TextKind::QuestSummary,
        TextKind::Journal,
    ];

    /// What this kind is called in a diagnostic addressed to a person.
    pub fn noun(self) -> &'static str {
        match self {
            TextKind::Bark => "bark",
            TextKind::Ambient => "ambient exchange",
            TextKind::Reaction => "companion reaction",
            TextKind::Codex => "codex entry",
            TextKind::QuestSummary => "quest summary",
            TextKind::Journal => "journal entry",
        }
    }

    /// Who is allowed to voice this kind.
    pub fn voice(self) -> Voice {
        match self {
            TextKind::Bark | TextKind::Ambient | TextKind::Reaction => Voice::Cast,
            TextKind::Codex | TextKind::QuestSummary | TextKind::Journal => Voice::Standalone,
        }
    }

    /// How many lines one delivery of this kind contains.
    pub fn delivery(self) -> Delivery {
        match self {
            // A bark and a reaction are one line by definition. Two lines would
            // be an exchange, and an exchange is an ambient asset, which the
            // host schedules and interrupts differently.
            TextKind::Bark | TextKind::Reaction => Delivery::Line,
            TextKind::Ambient => Delivery::Exchange,
            // Prose is a sequence for the ordinary reason prose is: paragraphs.
            TextKind::Codex | TextKind::QuestSummary | TextKind::Journal => Delivery::Passage,
        }
    }

    /// The selection policy a newly created asset of this kind starts with.
    ///
    /// A default rather than a rule, because it is a taste decision and the
    /// author owns it: barks and reactions rotate so the player does not hear
    /// the same one twice in a row, and prose shows whichever entry currently
    /// fits the world.
    pub fn default_repeat(self) -> RepeatPolicy {
        match self {
            TextKind::Bark | TextKind::Ambient | TextKind::Reaction => RepeatPolicy::Shuffle,
            TextKind::Codex | TextKind::QuestSummary | TextKind::Journal => RepeatPolicy::First,
        }
    }
}

/// Who may be the speaker of a supporting line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Voice {
    /// Characters in the asset's cast, the player, or the narrator. A cast
    /// member has to be listed in [`TextAsset::participants`] for the same
    /// reason a scene's speaker has to be a participant: otherwise the
    /// generation request has no voice description to work from and the export
    /// has no entity to attribute the recording to.
    Cast,
    /// The narrator or the player, and nobody else. A codex page attributed to
    /// a character would be that character *saying* it, which is a different
    /// asset with different review and recording consequences.
    Standalone,
}

impl Voice {
    /// Whether this voice admits the given speaker.
    pub fn admits(self, speaker: &Speaker) -> bool {
        match self {
            Voice::Cast => true,
            Voice::Standalone => speaker.entity().is_none(),
        }
    }
}

/// How many lines one delivery of an asset contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// Exactly one line.
    Line,
    /// An ordered sequence, and the whole point of the kind: consecutive lines
    /// are expected to have *different* speakers, which is what makes it an
    /// exchange rather than a monologue.
    Exchange,
    /// An ordered sequence of prose blocks with one voice.
    Passage,
}

impl Delivery {
    /// Whether an entry with this many lines is well formed.
    pub fn admits(self, lines: usize) -> bool {
        match self {
            Delivery::Line => lines == 1,
            Delivery::Exchange | Delivery::Passage => lines >= 1,
        }
    }
}

/// How the runtime picks an entry, and what happens on the next trigger.
///
/// Recorded in source rather than left to the host, because two hosts choosing
/// differently would make the same project play differently, and because the
/// deterministic-content promise (#173–#176) requires the engine adapter to have
/// no policy of its own. The rules themselves are defined with the runtime,
/// which is the only place that can hold the play counts they depend on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatPolicy {
    /// Always the first eligible entry in author order.
    ///
    /// The default, and the right answer for prose whose wording is a function
    /// of world state: a quest summary should show the entry that matches where
    /// the player is, every time it is read, not a different one each time.
    #[default]
    First,
    /// Each eligible entry once, in author order, and then nothing.
    ///
    /// For text that is exhausted rather than repeated — a journal that gains a
    /// new page per visit and then has no more to say.
    Once,
    /// Round-robin through the eligible entries in author order, wrapping.
    Cycle,
    /// A shuffled bag: every eligible entry once per round, in an order derived
    /// from the playthrough seed, never opening a round with the entry that
    /// closed the previous one.
    ///
    /// What a bark actually needs. `Cycle` is deterministic but audibly
    /// mechanical, and an unseeded random pick can say the same line three times
    /// running and cannot be reproduced from a save.
    Shuffle,
}

/// The host-side moment an asset belongs to.
///
/// `event` is a [`Name`] for the same reason a [`HostCommand`](crate::HostCommand)
/// name is one: it crosses into an engine that has to parse it without a Unicode
/// table, and it is compared for equality by an adapter Wobu has never seen.
///
/// The narrative does not observe *whether* the host raised the event, and
/// cannot: the asymmetry is the same one host commands have, and it is what lets
/// a game bind `player_enters_market` to whatever its own code means by that
/// without Wobu having an opinion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Trigger {
    pub event: Name,
    /// When the asset is eligible at all. `None` is unconditional, and is a
    /// different statement from `Some(Condition::Never)` — an author holding an
    /// asset back deliberately — for the same reason a scene's entry condition
    /// keeps the two apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
}

impl Trigger {
    pub fn on(event: Name) -> Trigger {
        Trigger { event, when: None }
    }
}

/// An authored record this text is about.
///
/// The "relevant source links" of US-09, and they earn their place twice: a
/// writer opening a bark wants to see the quest it belongs to, and a generation
/// request needs to know which world records to put in front of the model. They
/// are also the edges #168 will follow to decide that changing a fact made a
/// codex page stale, which is why they are typed links rather than prose
/// mentioning a quest by name.
///
/// A [`SceneId`] is admitted and a [`TextAssetId`] is not: an asset can be
/// *about* a scene, but one supporting asset citing another would be a
/// dependency cycle waiting to happen with no reader that benefits from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceLink {
    Scene(SceneId),
    Quest(EntityId),
    Fact(EntityId),
    Event(EntityId),
    Character(EntityId),
}

/// One alternative delivery of an asset.
///
/// A bark asset holds one entry per line the guard might say; an ambient asset
/// holds one entry per version of the exchange; a codex asset holds one entry
/// per state of the world it describes. In every case the entry is the unit the
/// selection policy chooses between, and the unit whose condition decides
/// whether it may be chosen at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct TextEntry {
    pub id: TextEntryId,
    /// The entry's name in the editor. A display string; nothing is derived
    /// from it and renaming it preserves every id underneath.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// When this entry may be chosen. `None` means always eligible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    /// The lines, in delivery order.
    ///
    /// A `Vec<DialogueSlot>` and not a `Vec<Text>`, which is the load-bearing
    /// reuse in this module: each line keeps its own slot identity, its own
    /// conditioned variants, and its own generation policy, so a two-speaker
    /// ambient exchange is two independently reviewable, independently
    /// recordable, independently translatable lines rather than one paragraph
    /// with a name in front of it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<DialogueSlot>,
}

impl TextEntry {
    pub fn new(label: impl Into<String>) -> TextEntry {
        TextEntry { id: TextEntryId::new(), label: label.into(), when: None, lines: Vec::new() }
    }

    /// A copy with fresh identities all the way down.
    ///
    /// Unlike [`Scene::duplicate_beat`](crate::Scene::duplicate_beat) there is
    /// nothing to rewire, because an entry has no destinations — which is the
    /// whole reason duplicating a bark is the safe, obvious operation that
    /// duplicating a beat is not.
    pub fn duplicated(&self) -> TextEntry {
        TextEntry {
            id: TextEntryId::new(),
            label: self.label.clone(),
            when: self.when.clone(),
            lines: self.lines.iter().map(DialogueSlot::duplicated).collect(),
        }
    }

    /// Whether a generation job may write anywhere in this entry.
    pub fn may_generate(&self) -> bool {
        self.lines.iter().all(DialogueSlot::may_generate)
    }
}

/// One supporting text asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct TextAsset {
    /// Head of the same immutable editorial history used by scene dialogue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editorial_head: Option<wobu_core::Id>,
    pub id: TextAssetId,
    pub kind: TextKind,
    /// The display name. Renaming preserves every id in the file.
    pub name: String,
    /// What the asset is for, in the author's words. Prose, and inert: it is a
    /// brief for a writer and material for a generation request, and nothing
    /// branches on it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    pub trigger: Trigger,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceLink>,
    /// The cast. Empty for standalone prose, and required to contain every
    /// entity that speaks a line — see [`Voice`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<Participant>,
    /// What every entry has to get across. Prose, and inert.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_convey: Vec<String>,
    /// What must not be revealed. Also prose, also inert, and worth the same
    /// bluntness as [`Beat::must_not_reveal`](crate::Beat::must_not_reveal): a
    /// sentence in this list is an instruction in a generation request, not a
    /// proof that a model never said the thing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_not_reveal: Vec<String>,
    #[serde(default)]
    pub repeat: RepeatPolicy,
    /// Whether generation may write into this asset at all.
    ///
    /// The asset-level counterpart of [`DialogueSlot::policy`], and locked for
    /// the same reason: US-05 requires a lock to hold for a caller that never
    /// went near the UI, which means it has to be checkable without walking
    /// every line first.
    #[serde(default)]
    pub policy: GenerationPolicy,
    /// Entries in author order. The order is the `Vec`; there is no `order`
    /// field on [`TextEntry`], because a stored index can disagree with the
    /// position it is stored at and then two readers disagree about which bark
    /// comes first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<TextEntry>,
    /// Deletions, kept so a locale row or a recording filed against a removed
    /// line can still be explained. See [`Tombstone`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tombstones: Vec<Tombstone>,
}

impl TextAsset {
    /// A new, empty asset of a kind, with that kind's default selection policy.
    pub fn new(kind: TextKind, name: impl Into<String>, event: Name) -> TextAsset {
        TextAsset {
            editorial_head: None,
            id: TextAssetId::new(),
            kind,
            name: name.into(),
            summary: String::new(),
            trigger: Trigger::on(event),
            sources: Vec::new(),
            participants: Vec::new(),
            must_convey: Vec::new(),
            must_not_reveal: Vec::new(),
            repeat: kind.default_repeat(),
            policy: GenerationPolicy::default(),
            entries: Vec::new(),
            tombstones: Vec::new(),
        }
    }

    pub fn entry(&self, id: TextEntryId) -> Option<&TextEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn entry_mut(&mut self, id: TextEntryId) -> Option<&mut TextEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub fn entry_index(&self, id: TextEntryId) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    /// Move an entry to a new position, keeping every id.
    ///
    /// It changes behaviour, and deliberately: entry order *is* the priority
    /// under [`RepeatPolicy::First`] and [`RepeatPolicy::Once`], and is the
    /// authored order a shuffle is derived from. That is the one place this
    /// model differs from beats, where reordering is inert, so it is stated
    /// here rather than left to be discovered.
    pub fn reorder_entry(&mut self, id: TextEntryId, to: usize) -> bool {
        let Some(from) = self.entry_index(id) else { return false };
        let to = to.min(self.entries.len().saturating_sub(1));
        let entry = self.entries.remove(from);
        self.entries.insert(to, entry);
        true
    }

    /// Copy an entry in beside the original, with fresh identities.
    pub fn duplicate_entry(&mut self, id: TextEntryId) -> Option<TextEntryId> {
        let index = self.entry_index(id)?;
        let copy = self.entries[index].duplicated();
        let new_id = copy.id;
        self.entries.insert(index + 1, copy);
        Some(new_id)
    }

    /// Remove an entry, leaving tombstones for it and for every line and wording
    /// under it that something outside this file might still name.
    pub fn remove_entry(&mut self, id: TextEntryId, reason: Option<String>) -> Option<TextEntry> {
        let index = self.entry_index(id)?;
        let entry = self.entries.remove(index);
        let deleted_at = Utc::now();
        let label = if entry.label.is_empty() { self.name.clone() } else { entry.label.clone() };

        self.tombstones.push(Tombstone {
            target: TombstoneTarget::TextEntry(entry.id),
            label: label.clone(),
            deleted_at,
            reason: reason.clone(),
        });
        for slot in &entry.lines {
            self.tombstones.push(Tombstone {
                target: TombstoneTarget::DialogueSlot(slot.id),
                label: label.clone(),
                deleted_at,
                reason: reason.clone(),
            });
            for variant in &slot.variants {
                self.tombstones.push(Tombstone {
                    target: TombstoneTarget::Variant(variant.id),
                    // The words themselves, because that is what identifies a
                    // line to the person holding a recording script.
                    label: variant.text.body.clone(),
                    deleted_at,
                    reason: reason.clone(),
                });
            }
        }
        Some(entry)
    }

    /// The tombstone explaining a production reference that no longer resolves.
    pub fn tombstone_for(&self, target: TombstoneTarget) -> Option<&Tombstone> {
        self.tombstones.iter().find(|t| t.target == target)
    }

    /// Copy the whole asset: a new asset id and new ids throughout.
    ///
    /// Tombstones are not carried over — they explain deletions that happened
    /// to the original, and the copy has no history yet — matching
    /// [`Scene::duplicated`](crate::Scene::duplicated).
    pub fn duplicated(&self) -> TextAsset {
        TextAsset {
            editorial_head: None,
            id: TextAssetId::new(),
            entries: self.entries.iter().map(TextEntry::duplicated).collect(),
            tombstones: Vec::new(),
            ..self.clone()
        }
    }

    /// Every line in the asset, with the entry it belongs to.
    ///
    /// The iterator the review queue, the locale pack and the recording script
    /// all walk, and the counterpart of
    /// [`Scene::dialogue_slots`](crate::Scene::dialogue_slots).
    pub fn lines(&self) -> impl Iterator<Item = (TextEntryId, &DialogueSlot)> {
        self.entries.iter().flat_map(|entry| entry.lines.iter().map(move |slot| (entry.id, slot)))
    }

    /// Whether a generation job may write anywhere in this asset.
    pub fn may_generate(&self) -> bool {
        self.policy != GenerationPolicy::Locked && self.entries.iter().all(TextEntry::may_generate)
    }

    /// Every entity that speaks somewhere in this asset.
    pub fn speakers(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.lines().filter_map(|(_, slot)| slot.speaker.entity())
    }

    /// Everything wrong with this asset that can be settled by reading it
    /// against the declared state.
    ///
    /// A list rather than a `Result`, for the reason
    /// [`Scene::diagnostics`](crate::Scene::diagnostics) gives: half-written is
    /// the normal state of authored content. Order is stable — the asset, then
    /// entries in author order — so a rendered list does not jump between runs.
    pub fn diagnostics(&self, schema: &StateSchema) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let kind = self.kind.noun();

        if self.entries.is_empty() {
            out.push(Diagnostic { site: Site::TextAsset, problem: Problem::NoTextEntries });
        }
        if let Some(when) = &self.trigger.when
            && let Err(err) = schema.check_condition(when)
        {
            out.push(Diagnostic { site: Site::TextTrigger, problem: err.into() });
        }
        if self.kind.voice() == Voice::Standalone && !self.participants.is_empty() {
            out.push(Diagnostic { site: Site::TextAsset, problem: Problem::ProseHasCast { kind } });
        }
        self.check_unique_ids(&mut out);

        let cast: Vec<EntityId> = self.participants.iter().map(|p| p.entity).collect();
        for entry in &self.entries {
            let site = Site::TextEntry { entry: entry.id };
            if let Some(when) = &entry.when
                && let Err(err) = schema.check_condition(when)
            {
                out.push(Diagnostic { site, problem: err.into() });
            }
            if !self.kind.delivery().admits(entry.lines.len()) {
                out.push(Diagnostic {
                    site,
                    problem: Problem::WrongLineCount { kind, found: entry.lines.len() },
                });
            }
            for slot in &entry.lines {
                let site = Site::TextSlot { entry: entry.id, slot: slot.id };
                if !self.kind.voice().admits(&slot.speaker) {
                    out.push(Diagnostic { site, problem: Problem::VoiceNotAllowed { kind } });
                } else if let Some(entity) = slot.speaker.entity()
                    && !cast.contains(&entity)
                {
                    out.push(Diagnostic { site, problem: Problem::NotAParticipant { entity } });
                }
                if slot.is_missing_text() {
                    out.push(Diagnostic { site, problem: Problem::MissingText });
                }
                for variant in &slot.variants {
                    let site =
                        Site::TextVariant { entry: entry.id, slot: slot.id, variant: variant.id };
                    if let Some(when) = &variant.when
                        && let Err(err) = schema.check_condition(when)
                    {
                        out.push(Diagnostic { site, problem: err.into() });
                    }
                    if !variant.text.revision_matches() {
                        out.push(Diagnostic {
                            site,
                            problem: Problem::RevisionMismatch {
                                recorded: variant.text.revision.clone(),
                                computed: variant.text.computed_revision(),
                            },
                        });
                    }
                }
            }
        }
        out
    }

    /// Ids have to be unique within the asset, for the reason
    /// [`Scene`](crate::Scene) checks the same thing: everything downstream —
    /// the locale row, the recording script, the compiled string table — indexes
    /// by these, and a silent duplicate means one row overwriting another.
    fn check_unique_ids(&self, out: &mut Vec<Diagnostic>) {
        let mut entries: Vec<TextEntryId> = Vec::new();
        let mut slots: Vec<DialogueSlotId> = Vec::new();
        let mut variants: Vec<VariantId> = Vec::new();
        for entry in &self.entries {
            if entries.contains(&entry.id) {
                out.push(Diagnostic {
                    site: Site::TextEntry { entry: entry.id },
                    problem: Problem::DuplicateId {
                        noun: TextEntryId::NOUN,
                        id: entry.id.to_string(),
                    },
                });
            }
            entries.push(entry.id);
            for slot in &entry.lines {
                if slots.contains(&slot.id) {
                    out.push(Diagnostic {
                        site: Site::TextSlot { entry: entry.id, slot: slot.id },
                        problem: Problem::DuplicateId {
                            noun: DialogueSlotId::NOUN,
                            id: slot.id.to_string(),
                        },
                    });
                }
                slots.push(slot.id);
                for variant in &slot.variants {
                    if variants.contains(&variant.id) {
                        out.push(Diagnostic {
                            site: Site::TextVariant {
                                entry: entry.id,
                                slot: slot.id,
                                variant: variant.id,
                            },
                            problem: Problem::DuplicateId {
                                noun: VariantId::NOUN,
                                id: variant.id.to_string(),
                            },
                        });
                    }
                    variants.push(variant.id);
                }
            }
        }
    }
}

/// One supporting text asset as a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct TextAssetDocument {
    pub schema_version: u32,
    pub asset: TextAsset,
}

impl TextAssetDocument {
    pub fn new(asset: TextAsset) -> TextAssetDocument {
        TextAssetDocument { schema_version: TEXT_SCHEMA_VERSION, asset }
    }

    pub fn parse(yaml: &str) -> crate::Result<TextAssetDocument> {
        crate::source::check_version_for(yaml, TEXT_SCHEMA_VERSION)?;
        crate::source::parse_yaml(yaml)
    }

    pub fn to_yaml(&self) -> crate::Result<String> {
        if self.schema_version != TEXT_SCHEMA_VERSION {
            return Err(crate::Error::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: TEXT_SCHEMA_VERSION,
            });
        }
        crate::source::print_yaml(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::ReviewState;
    use crate::scene::{Text, Variant};
    use crate::state::{Owner, Value, VarType, VariableDecl};

    fn name(value: &str) -> Name {
        Name::new(value).unwrap()
    }

    fn schema() -> StateSchema {
        StateSchema::new([VariableDecl {
            name: name("alarm_raised"),
            ty: VarType::Bool,
            default: Value::Bool(false),
            owner: Owner::Narrative,
            description: String::new(),
        }])
        .unwrap()
    }

    fn spoken(speaker: Speaker, body: &str) -> DialogueSlot {
        let mut slot = DialogueSlot::new(speaker);
        slot.variants.push(Variant::new(Text::written(body)));
        slot
    }

    fn bark(entity: EntityId) -> TextAsset {
        let mut asset = TextAsset::new(TextKind::Bark, "Gate guard", name("player_passes_gate"));
        asset.participants.push(Participant { entity, role: String::new() });
        let mut entry = TextEntry::new("Move along");
        entry.lines.push(spoken(Speaker::Entity(entity), "Move along."));
        asset.entries.push(entry);
        asset
    }

    #[test]
    fn the_six_kinds_split_into_cast_and_standalone() {
        let cast: Vec<_> =
            TextKind::ALL.iter().filter(|k| k.voice() == Voice::Cast).copied().collect();
        assert_eq!(cast, vec![TextKind::Bark, TextKind::Ambient, TextKind::Reaction]);
    }

    #[test]
    fn a_well_formed_bark_has_no_diagnostics() {
        assert!(bark(wobu_core::new_id()).diagnostics(&schema()).is_empty());
    }

    #[test]
    fn an_asset_with_no_entries_says_so() {
        let asset = TextAsset::new(TextKind::Codex, "Beacons", name("codex_opened"));
        let problems = asset.diagnostics(&schema());
        assert!(matches!(problems[0].problem, Problem::NoTextEntries), "{problems:?}");
    }

    #[test]
    fn standalone_prose_cannot_be_voiced_by_a_character() {
        let entity = wobu_core::new_id();
        let mut asset = TextAsset::new(TextKind::Codex, "Beacons", name("codex_opened"));
        let mut entry = TextEntry::new("Overview");
        entry.lines.push(spoken(Speaker::Entity(entity), "The beacons are older than the city."));
        asset.entries.push(entry);
        let problems = asset.diagnostics(&schema());
        assert!(
            problems.iter().any(|d| matches!(d.problem, Problem::VoiceNotAllowed { .. })),
            "{problems:?}"
        );
    }

    #[test]
    fn a_journal_may_be_written_in_the_players_voice() {
        let mut asset = TextAsset::new(TextKind::Journal, "Day one", name("day_ends"));
        let mut entry = TextEntry::new("Day one");
        entry.lines.push(spoken(Speaker::Player, "I found the logbook."));
        asset.entries.push(entry);
        assert!(asset.diagnostics(&schema()).is_empty());
    }

    #[test]
    fn a_bark_is_one_line_and_an_exchange_is_not() {
        let entity = wobu_core::new_id();
        let mut asset = bark(entity);
        asset.entries[0].lines.push(spoken(Speaker::Entity(entity), "I said move."));
        let problems = asset.diagnostics(&schema());
        assert!(
            problems.iter().any(|d| matches!(d.problem, Problem::WrongLineCount { found: 2, .. })),
            "{problems:?}"
        );

        // The same two lines are exactly what an ambient exchange is for.
        asset.kind = TextKind::Ambient;
        assert!(asset.diagnostics(&schema()).is_empty());
    }

    #[test]
    fn an_ambient_exchange_carries_several_speakers_as_separate_lines() {
        let dockhand = wobu_core::new_id();
        let mate = wobu_core::new_id();
        let mut asset = TextAsset::new(TextKind::Ambient, "Dock argument", name("market_idle"));
        asset.participants.push(Participant { entity: dockhand, role: "dockhand".into() });
        asset.participants.push(Participant { entity: mate, role: "mate".into() });
        let mut entry = TextEntry::new("Short tally");
        entry.lines.push(spoken(Speaker::Entity(dockhand), "Tally's short again."));
        entry.lines.push(spoken(Speaker::Entity(mate), "Then count it twice."));
        asset.entries.push(entry);

        assert!(asset.diagnostics(&schema()).is_empty());
        // Two lines, two identities: each is separately reviewable, recordable
        // and translatable, which a single joined paragraph could not be.
        let slots: Vec<_> = asset.lines().map(|(_, slot)| slot.id).collect();
        assert_eq!(slots.len(), 2);
        assert_ne!(slots[0], slots[1]);
    }

    #[test]
    fn a_speaker_outside_the_cast_is_reported() {
        let mut asset = bark(wobu_core::new_id());
        asset.entries[0].lines[0].speaker = Speaker::Entity(wobu_core::new_id());
        let problems = asset.diagnostics(&schema());
        assert!(
            problems.iter().any(|d| matches!(d.problem, Problem::NotAParticipant { .. })),
            "{problems:?}"
        );
    }

    #[test]
    fn a_condition_over_an_undeclared_variable_is_reported_once_per_site() {
        let mut asset = bark(wobu_core::new_id());
        let bad = Condition::Compare(crate::expr::Comparison {
            var: name("undeclared"),
            op: crate::expr::CompareOp::Eq,
            value: crate::expr::Operand::Literal(Value::Bool(true)),
        });
        asset.trigger.when = Some(bad.clone());
        asset.entries[0].when = Some(bad.clone());
        asset.entries[0].lines[0].variants[0].when = Some(bad);
        let sites: Vec<_> = asset.diagnostics(&schema()).into_iter().map(|d| d.site).collect();
        assert!(sites.contains(&Site::TextTrigger), "{sites:?}");
        assert!(sites.iter().any(|s| matches!(s, Site::TextEntry { .. })), "{sites:?}");
        assert!(sites.iter().any(|s| matches!(s, Site::TextVariant { .. })), "{sites:?}");
    }

    #[test]
    fn duplicating_an_entry_mints_new_identities_and_drafts_the_copy() {
        let mut asset = bark(wobu_core::new_id());
        asset.entries[0].lines[0].variants[0].text.lifecycle.review = ReviewState::Approved;
        let original = asset.entries[0].lines[0].variants[0].id;
        let copy = asset.duplicate_entry(asset.entries[0].id).unwrap();

        assert_eq!(asset.entries.len(), 2);
        assert_ne!(asset.entries[1].id, asset.entries[0].id);
        assert_eq!(asset.entries[1].id, copy);
        assert_ne!(asset.entries[1].lines[0].variants[0].id, original);
        // An approval was given to the original line; nobody has looked at the
        // copy, so it must not arrive pre-approved.
        assert_eq!(asset.entries[1].lines[0].variants[0].text.lifecycle.review, ReviewState::Draft);
    }

    #[test]
    fn removing_an_entry_leaves_tombstones_for_everything_production_can_name() {
        let mut asset = bark(wobu_core::new_id());
        let entry = asset.entries[0].id;
        let slot = asset.entries[0].lines[0].id;
        let variant = asset.entries[0].lines[0].variants[0].id;

        asset.remove_entry(entry, Some("cut in review".into())).unwrap();
        assert!(asset.entries.is_empty());
        assert!(asset.tombstone_for(TombstoneTarget::TextEntry(entry)).is_some());
        assert!(asset.tombstone_for(TombstoneTarget::DialogueSlot(slot)).is_some());
        let words = asset.tombstone_for(TombstoneTarget::Variant(variant)).unwrap();
        assert_eq!(words.label, "Move along.");
    }

    #[test]
    fn a_locked_asset_refuses_generation_without_walking_its_lines() {
        let mut asset = bark(wobu_core::new_id());
        assert!(asset.may_generate());
        asset.policy = GenerationPolicy::Locked;
        assert!(!asset.may_generate());
    }

    #[test]
    fn a_locked_line_refuses_generation_for_the_whole_asset() {
        let mut asset = bark(wobu_core::new_id());
        asset.entries[0].lines[0].policy = GenerationPolicy::Locked;
        assert!(!asset.may_generate());
    }

    #[test]
    fn a_document_round_trips_through_yaml() {
        let asset = bark(wobu_core::new_id());
        let yaml = TextAssetDocument::new(asset.clone()).to_yaml().unwrap();
        assert_eq!(TextAssetDocument::parse(&yaml).unwrap().asset, asset);
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_dropped() {
        let asset = bark(wobu_core::new_id());
        let yaml = TextAssetDocument::new(asset).to_yaml().unwrap();
        let tampered = yaml.replace("repeat:", "repaet:");
        assert!(TextAssetDocument::parse(&tampered).is_err());
    }

    #[test]
    fn a_newer_schema_version_is_refused_before_its_shape_is_read() {
        let err = TextAssetDocument::parse("schema_version: 99\nasset: {}\n").unwrap_err();
        assert!(matches!(err, crate::Error::UnsupportedSchemaVersion { found: 99, .. }), "{err}");
    }
}
