//! Stable identities, and the content revisions that are deliberately not
//! identities.
//!
//! The whole narrative system rests on one distinction, so it is worth stating
//! plainly before any of the types below:
//!
//! - An **id** names a *slot*. It is minted once, from nothing but the clock and
//!   a random source, and it never changes again. Renaming a beat, reordering
//!   the beats around it, rewriting every line inside it, moving it on a canvas
//!   — none of those touch the id. Copying it mints a new one, because a copy is
//!   a different slot that happens to start with the same contents.
//! - A **revision** names *wording*. It is derived, from the text and the
//!   provenance of that text, and it changes on every edit. It is never minted
//!   and never chosen.
//!
//! The types enforce the split rather than merely describing it: every id has a
//! `new()` and no way to derive one from content, and [`Revision`] has a
//! [`Revision::of`] and no constructor at all. There is no operation that turns
//! one into the other.
//!
//! Getting this backwards is the failure the rest of the system cannot recover
//! from. Ids derived from content would change when a writer fixed a typo,
//! silently orphaning the translation, the recorded audio and the lip-sync data
//! keyed to the line (#178, #179). Revisions minted at random would make it
//! impossible to say whether the wording a reviewer approved is the wording
//! about to ship, which is the check the whole review flow is built on.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Declare one stable identity type.
///
/// Each is a distinct Rust type rather than a shared `Id` alias, because the
/// most valuable check this crate can offer is the one that happens before the
/// program runs: a destination that points at a [`SceneId`] where a [`BeatId`]
/// belongs is a mistake the compiler should catch, not a diagnostic somebody
/// reads after the file is saved.
macro_rules! narrative_id {
    ($(#[$doc:meta])* $name:ident, $noun:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(wobu_core::Id);

        impl $name {
            /// Mint a fresh identity.
            ///
            /// The only way to obtain one. Duplication calls this; renaming and
            /// reordering must not.
            pub fn new() -> $name {
                $name(wobu_core::new_id())
            }

            /// The underlying ULID, for callers that have to index or sort by it.
            pub fn raw(self) -> wobu_core::Id {
                self.0
            }

            /// Rebuild an identity that already exists — reading a file, or
            /// taking one back over an IPC boundary. Not a way to invent one:
            /// the caller has to already hold the ULID.
            pub fn from_raw(id: wobu_core::Id) -> $name {
                $name(id)
            }

            /// What this identity is called in a diagnostic addressed to a
            /// person: "beat", "choice", and so on.
            pub const NOUN: &'static str = $noun;
        }

        /// The same thing as [`new`](Self::new), and it has to be, because
        /// clippy asks for a `Default` beside every `new`. Minting rather than
        /// producing a zero id is the only defensible answer: a shared "empty"
        /// id would compare equal to every other empty one, and two beats that
        /// forgot to set theirs would silently become the same beat.
        impl Default for $name {
            fn default() -> $name {
                $name::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(s: &str) -> Result<$name> {
                wobu_core::Id::from_str(s)
                    .map($name)
                    .map_err(|_| Error::InvalidId { noun: $noun, value: s.to_string() })
            }
        }
    };
}

narrative_id!(
    /// One authored scene.
    SceneId,
    "scene"
);
narrative_id!(
    /// One beat within a scene: a unit of intent, dialogue and branching.
    BeatId,
    "beat"
);
narrative_id!(
    /// One player-facing choice offered by a beat.
    ChoiceId,
    "choice"
);
narrative_id!(
    /// One automatic transition out of a beat.
    OutcomeId,
    "outcome"
);
narrative_id!(
    /// One named place where a line of dialogue goes.
    DialogueSlotId,
    "dialogue slot"
);
narrative_id!(
    /// One conditioned wording for a dialogue slot.
    ///
    /// A slot with seven variants is still one slot: the variants multiply
    /// content, not structure. Each carries its own identity because
    /// localisation and audio key on the individual wording, not on the slot.
    VariantId,
    "variant"
);

/// Where a piece of wording came from.
///
/// Half of what a [`Revision`] hashes, and the reason two identical strings can
/// legitimately carry different revisions: "the writer typed this" and "a model
/// produced this and nobody has looked at it yet" are different facts about the
/// same sentence, and the review queue has to be able to tell them apart.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Provenance {
    /// Typed by a person. The default, because this crate has to be usable —
    /// and a whole scene has to be authorable — without any provider
    /// configured at all (US-03).
    #[default]
    Human,
    /// Produced by a generation job. `fingerprint` is an opaque handle to the
    /// request that produced it; what actually goes into that fingerprint —
    /// resolved context, prompt schema version, model identity — is decided
    /// where generation lives (#163), not here. This crate only has to carry it
    /// so the revision changes when the inputs did.
    Generated { fingerprint: String },
    /// Brought in from outside Wobu — an import, a migration, a paste from a
    /// previous tool. Recorded rather than flattened into `Human` so a reviewer
    /// is not told a person wrote something nobody in this project ever read.
    Imported { source: String },
}

impl Provenance {
    fn tag(&self) -> &'static str {
        match self {
            Provenance::Human => "human",
            Provenance::Generated { .. } => "generated",
            Provenance::Imported { .. } => "imported",
        }
    }

    fn detail(&self) -> &str {
        match self {
            Provenance::Human => "",
            Provenance::Generated { fingerprint } => fingerprint,
            Provenance::Imported { source } => source,
        }
    }
}

/// The domain separator mixed into every revision hash.
///
/// Present so a revision can never collide with one of the other blake3 digests
/// this project stores — asset content hashes, index versions — and versioned so
/// that if the recipe below ever has to change, old revisions stay recognisable
/// as old rather than quietly appearing to be new ones.
const REVISION_DOMAIN: &[u8] = b"wobu-narrative/revision/1";

/// A content hash over wording and provenance.
///
/// Not an identity. See the module documentation: this changes whenever the
/// text does, which is exactly what makes it useful as the thing an approval,
/// a translation and a recording all attest to.
///
/// Deliberately has no constructor. The only way to obtain one is
/// [`Revision::of`], so there is no code path anywhere in the workspace that
/// can invent a revision that does not describe some actual text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision(String);

impl Revision {
    /// Derive the revision of some wording.
    ///
    /// Lengths are hashed alongside the bytes so that splitting the same
    /// characters differently between the text and its provenance cannot
    /// produce the same digest — otherwise a line reading `ab` from source `c`
    /// and a line reading `a` from source `bc` would be indistinguishable, and
    /// a stale translation could pass a revision check.
    pub fn of(text: &str, provenance: &Provenance) -> Revision {
        let mut hasher = blake3::Hasher::new();
        hasher.update(REVISION_DOMAIN);
        for part in [text, provenance.tag(), provenance.detail()] {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        }
        // Half a blake3 digest. 128 bits is far past any collision risk for the
        // number of lines a game can contain, and the short form is what makes a
        // recording script or a locale row readable by the person holding it.
        Revision(hasher.finalize().to_hex()[..32].to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct_every_time() {
        assert_ne!(BeatId::new(), BeatId::new());
    }

    #[test]
    fn ids_round_trip_through_their_string_form() {
        let id = SceneId::new();
        assert_eq!(SceneId::from_str(&id.to_string()).unwrap(), id);
    }

    #[test]
    fn a_malformed_id_names_what_it_was_meant_to_be() {
        // "invalid ULID" tells an author nothing; "not a valid beat id" points
        // at the field they mistyped.
        let err = BeatId::from_str("not-a-ulid").unwrap_err();
        assert!(err.to_string().contains("beat"), "{err}");
    }

    #[test]
    fn a_revision_follows_the_text() {
        let a = Revision::of("Show them the logbook.", &Provenance::Human);
        let b = Revision::of("Show them the logbook", &Provenance::Human);
        assert_ne!(a, b);
        assert_eq!(a, Revision::of("Show them the logbook.", &Provenance::Human));
    }

    #[test]
    fn a_revision_follows_the_provenance_too() {
        // Same sentence, different story about where it came from. The review
        // queue has to be able to tell these apart.
        let typed = Revision::of("I was there.", &Provenance::Human);
        let drafted =
            Revision::of("I was there.", &Provenance::Generated { fingerprint: "job-7".into() });
        assert_ne!(typed, drafted);
    }

    #[test]
    fn the_split_between_text_and_provenance_is_hashed() {
        // Without the length prefixes these two would collide, and a locale row
        // keyed on one would silently validate against the other.
        let a = Revision::of("ab", &Provenance::Imported { source: "c".into() });
        let b = Revision::of("a", &Provenance::Imported { source: "bc".into() });
        assert_ne!(a, b);
    }
}
