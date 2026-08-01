//! The losers of a guarded write, and what a person can do about them.
//!
//! [`crate::atomic::guarded_write`] parks the version that lost a race beside
//! the one that won, as `<stem>.conflict-<peer>-<timestamp>[-n].<ext>`. This
//! module is the other half of that: reading those names back into something
//! the UI can put on a card, and carrying out the one decision a human makes
//! about them.
//!
//! `<peer>` is a short alias for the writer's ed25519 key ([`crate::peer`]),
//! which since #76 is what replaces the `$USER` that used to go there. The
//! difference that matters to *this* module is that the name means the same
//! thing on every machine, so a sibling written on somebody else's laptop and
//! synced onto ours is attributable rather than merely labelled.
//!
//! **Nothing here runs on a timer, a scan or a rebuild.** A conflict sibling is
//! the only remaining copy of somebody's paragraph, so the sole code path in
//! Wobu that removes one is [`crate::Project::resolve_conflict`], reached only
//! from a button the user pressed. `Project::sweep_staging` deliberately cannot
//! reach one — it is scoped to `.wobu/tmp` and to the `.part` extension —
//! and `rescan`/`rebuild_index` only ever read. See `docs/07-file-shares.md`.
//!
//! We do not merge prose, and the card does not offer to. Three sentences that
//! two people rewrote in different directions have no correct interleaving, and
//! a machine guessing at one produces text neither of them wrote.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use wobu_core::Id;

/// The infix `guarded_write` stamps into a losing write's filename.
pub const MARKER: &str = ".conflict-";

/// The timestamp format in a sibling's name, to the second.
///
/// Kept next to the parser rather than shared with `atomic`: these two are
/// deliberately a written contract between a writer and a reader that may be
/// different builds of Wobu on different machines, and a format string one side
/// can silently change is not a contract.
const STAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";

/// Whether a filename is one of ours.
///
/// The same test `Project::node_files` uses to keep siblings out of the
/// navigator, so the two can never disagree about what counts.
pub fn is_sibling(file_name: &str) -> bool {
    file_name.contains(MARKER)
}

/// `kael-vantris.conflict-amber-heron-4f1a-20260731T142211Z.md` taken apart.
///
/// `peer` and `saved_at` are optional because a name that does not parse is
/// still a file holding somebody's only copy of a paragraph. Refusing to list
/// one we cannot label would make it unresolvable from inside the app, which is
/// a worse outcome than a card that says "someone, at some point".
///
/// That optionality is also the migration story, and it is the whole of it: a
/// sibling written by a build from before #76 says `jake` where a new one says
/// `amber-heron-4f1a`, and both parse, because the parser only ever cared that
/// the fragment before the timestamp is a slug. Nothing renames anything — a
/// conflict sibling is somebody's only copy, and touching one to tidy up a name
/// is not a trade this crate makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiblingName {
    /// `kael-vantris` — the node file it lost to, minus the extension.
    pub stem: String,
    pub ext: Option<String>,
    /// The alias of whoever lost the race. Named `peer` rather than `user`
    /// because it is a key's short name and not a login: nothing on this machine
    /// can produce somebody else's, which was the point of #76.
    pub peer: Option<String>,
    pub saved_at: Option<DateTime<Utc>>,
}

impl SiblingName {
    /// The name of the file this sibling was parked beside.
    pub fn target_file_name(&self) -> String {
        match &self.ext {
            Some(ext) => format!("{}.{ext}", self.stem),
            None => self.stem.clone(),
        }
    }
}

/// Read a conflict sibling's filename back into its parts, or `None` if it is
/// not one.
pub fn parse(file_name: &str) -> Option<SiblingName> {
    // Split the extension off first, because the marker search below would
    // otherwise have to know whether `.md` was part of the timestamp run. A
    // name with no extension at all is legal — `guarded_write` produces one for
    // a target that had none.
    let (body, ext) = match file_name.rsplit_once('.') {
        Some((body, ext)) if !ext.contains(MARKER.trim_start_matches('.')) && body.contains(MARKER) => {
            (body, Some(ext.to_string()))
        }
        _ => (file_name, None),
    };

    // The last marker wins. A node whose own slug contained the word would
    // otherwise hand us its stem as the conflict metadata.
    let at = body.rfind(MARKER)?;
    let stem = &body[..at];
    let tail = &body[at + MARKER.len()..];
    if stem.is_empty() {
        return None;
    }

    // `<peer>-<timestamp>` with an optional `-<n>`, where the peer alias is a
    // slug and so may itself contain hyphens — `amber-heron-4f1a` is three
    // components on its own. The timestamp is the only part with a fixed shape,
    // so it is what the split is anchored on. `slugify` lowercases, which is why
    // an alias can never be mistaken for one: the `T` and `Z` are matched
    // literally.
    let parts: Vec<&str> = tail.split('-').collect();
    let stamp_at = parts.iter().position(|p| parse_stamp(p).is_some());

    let (peer, saved_at) = match stamp_at {
        Some(i) if i > 0 => (Some(parts[..i].join("-")), parse_stamp(parts[i])),
        // No timestamp, or nothing before it. Still a sibling; just an unlabelled one.
        _ => (None, None),
    };

    Some(SiblingName {
        stem: stem.to_string(),
        ext,
        peer: peer.filter(|p| !p.is_empty()),
        saved_at,
    })
}

fn parse_stamp(s: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(s, STAMP_FORMAT).ok().map(|n| n.and_utc())
}

/// An unresolved conflict, as the card renders it.
///
/// Both documents travel whole rather than as a diff. The diff is a rendering
/// decision — how much context, whether to fold the unchanged parts — and
/// computing it here would fix that decision in a build of the backend, on the
/// far side of a bridge, for a payload measured in kilobytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// The sibling, project-relative. Also the handle `resolve_conflict` takes.
    pub rel_path: String,
    /// The node file it was parked beside.
    pub node_rel_path: String,
    /// `None` when the node file itself is missing or unparseable — the sibling
    /// is still resolvable, it just cannot be named after an entity.
    pub node_id: Option<Id>,
    pub node_name: Option<String>,
    /// Who lost the race, from the filename: the peer alias, or `None` for a
    /// sibling whose name nothing here could read.
    ///
    /// Still called `user` on the wire, because it is what the card puts in
    /// front of a person and "user" is the word `src/components/ConflictCard.tsx`
    /// renders it under. The Rust side calls it a peer everywhere else; see
    /// [`SiblingName::peer`] for why the distinction is worth keeping.
    pub user: Option<String>,
    pub saved_at: Option<DateTime<Utc>>,
    /// Whether this installation's peer alias is the one stamped on the sibling
    /// — the difference between a card that says "keep mine" and one that says
    /// "keep Nadia's".
    ///
    /// Before #76 this was wrong-but-harmless on a share where two people shared
    /// a login. It is now wrong only when a machine could not reach its keychain
    /// and is running under the unattributed fallback, in which case it reads
    /// `false` for siblings that really were ours. Still nothing but the wording
    /// depends on it, and that is still on purpose: an alias is twenty-eight bits
    /// and may never decide anything.
    pub mine: bool,
    /// The text that was set aside.
    pub parked: String,
    /// What is at `node_rel_path` right now.
    pub current: String,
    /// Hash of `current`, handed back to `resolve_conflict` so a decision made
    /// against a version that has since been replaced is refused rather than
    /// applied to text the user never read.
    pub current_hash: String,
}

/// Which of the two versions the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Keep {
    /// The sibling wins: its text is written to the node file, then the sibling
    /// is removed.
    Parked,
    /// The node file wins: it is not touched, and the sibling is removed.
    Current,
}

/// What came of a resolution. Every variant but `Resolved` leaves both files
/// exactly where they were.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum Resolved {
    /// The choice landed and the losing file is gone.
    Done,
    /// The node file changed between the card being drawn and the button being
    /// pressed, so the diff the user answered is not the one on disk.
    ///
    /// Nothing is written and nothing is deleted. This is the third-writer case
    /// and it is the whole reason `current_hash` crosses the bridge: without it
    /// "keep theirs" would silently discard a version the user chose in favour
    /// of one they never saw.
    Stale,
    /// The write itself lost a race in the window between the check and the
    /// rename. Our pick is parked as a further sibling and the original is
    /// untouched, so no text is anywhere near being lost — there is just one
    /// more decision to make.
    #[serde(rename_all = "camelCase")]
    Conflict { conflict_path: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(name: &str) -> SiblingName {
        parse(name).unwrap_or_else(|| panic!("{name} should parse as a sibling"))
    }

    #[test]
    fn the_name_guarded_write_produces_round_trips() {
        // The one case that has to work; everything below is a variation on it.
        // The peer is an alias — three hyphenated components of its own, which
        // is the shape the parser has to survive since #76.
        let n = parsed("kael-vantris.conflict-amber-heron-4f1a-20260731T142211Z.md");
        assert_eq!(n.stem, "kael-vantris");
        assert_eq!(n.ext.as_deref(), Some("md"));
        assert_eq!(n.peer.as_deref(), Some("amber-heron-4f1a"));
        assert_eq!(n.saved_at.unwrap().to_rfc3339(), "2026-07-31T14:22:11+00:00");
        assert_eq!(n.target_file_name(), "kael-vantris.md");
    }

    #[test]
    fn a_name_a_pre_identity_build_wrote_still_parses() {
        // The migration, and the whole of it. A share that has been in use since
        // before #76 has `$USER` siblings sitting in it, and each one is still
        // somebody's only copy of a paragraph. Nothing renames them — they parse,
        // they list, and the card labels them with whatever login wrote them.
        let n = parsed("kael-vantris.conflict-jake-20260731T142211Z.md");
        assert_eq!(n.peer.as_deref(), Some("jake"));
        assert_eq!(n.target_file_name(), "kael-vantris.md");

        // Including the collision that made #76 worth doing: everyone on a
        // default install was `user`, and the file is still resolvable.
        let n = parsed("kael-vantris.conflict-user-20260731T142211Z.md");
        assert_eq!(n.peer.as_deref(), Some("user"));
    }

    #[test]
    fn a_peer_alias_with_hyphens_is_not_split_across_the_timestamp() {
        // Every alias has two of these — `amber-heron-4f1a` — so splitting on
        // the first hyphen would name the peer "amber" and lose the rest, which
        // is a card that attributes a paragraph to the wrong person. The old
        // `slugify("Nadia Okonkwo")` case is the same failure and is kept.
        let n = parsed("kael-vantris.conflict-nadia-okonkwo-20260731T142211Z.md");
        assert_eq!(n.peer.as_deref(), Some("nadia-okonkwo"));
        assert_eq!(n.stem, "kael-vantris");

        let n = parsed("kael-vantris.conflict-silver-plover-00ff-20260731T142211Z.md");
        assert_eq!(n.peer.as_deref(), Some("silver-plover-00ff"));
        assert_eq!(n.stem, "kael-vantris");
    }

    #[test]
    fn an_alias_whose_suffix_is_all_digits_is_not_read_as_a_timestamp() {
        // A quarter of aliases end in four hex characters that are entirely
        // numeric, and the split anchors on a numeric-looking component. The
        // format needs sixteen characters with a literal `T` and `Z`, so this
        // cannot land in the wrong place — restated as an assertion because the
        // failure is silent and misattributes somebody's writing.
        let n = parsed("kael-vantris.conflict-golden-otter-2026-20260731T142211Z.md");
        assert_eq!(n.peer.as_deref(), Some("golden-otter-2026"));
        assert_eq!(n.saved_at.unwrap().to_rfc3339(), "2026-07-31T14:22:11+00:00");
    }

    #[test]
    fn the_collision_suffix_is_not_read_as_part_of_the_timestamp() {
        // Two conflicts inside one second get `-2`, `-3`. Treating that as part
        // of the stamp would make the second one unparseable and therefore
        // unlabelled on the card.
        let n = parsed("kael-vantris.conflict-jake-20260731T142211Z-2.md");
        assert_eq!(n.peer.as_deref(), Some("jake"));
        assert_eq!(n.saved_at.unwrap().to_rfc3339(), "2026-07-31T14:22:11+00:00");
        assert_eq!(n.target_file_name(), "kael-vantris.md");
    }

    #[test]
    fn a_target_that_had_no_extension_still_parses() {
        let n = parsed("readme.conflict-jake-20260731T142211Z");
        assert_eq!(n.stem, "readme");
        assert_eq!(n.ext, None);
        assert_eq!(n.target_file_name(), "readme");
    }

    #[test]
    fn an_unreadable_name_is_still_a_sibling() {
        // A hand-made or half-synced name has to stay listable. Refusing it
        // would leave a file holding somebody's only copy with no way to
        // resolve it from inside the app.
        let n = parsed("kael-vantris.conflict-something-odd.md");
        assert_eq!(n.stem, "kael-vantris");
        assert_eq!(n.target_file_name(), "kael-vantris.md");
        assert_eq!(n.saved_at, None);
    }

    #[test]
    fn an_ordinary_node_file_is_not_a_sibling() {
        assert!(parse("kael-vantris.md").is_none());
        assert!(!is_sibling("kael-vantris.md"));
        assert!(is_sibling("kael-vantris.conflict-jake-20260731T142211Z.md"));
    }

    #[test]
    fn a_sibling_of_a_sibling_resolves_against_the_nearer_marker() {
        // Not something `guarded_write` produces — siblings are never node
        // files, so they are never written to — but a user copying files around
        // can make one, and the target it names has to be the file it sits
        // beside rather than the original node.
        let n = parsed("kael.conflict-jake-20260731T142211Z.conflict-nadia-20260731T150000Z.md");
        assert_eq!(n.peer.as_deref(), Some("nadia"));
        assert_eq!(n.stem, "kael.conflict-jake-20260731T142211Z");
    }

    #[test]
    fn a_lowercased_slug_cannot_be_mistaken_for_a_timestamp() {
        // The parser anchors on the timestamp's shape, so this is the failure
        // to guard: a user whose slug happened to look like one would have the
        // split land in the wrong place. `slugify` lowercases and the format
        // matches `T`/`Z` literally, so it cannot.
        let n = parsed("kael.conflict-20260731t142211z-20260731T142211Z.md");
        assert_eq!(n.peer.as_deref(), Some("20260731t142211z"));
    }

    #[test]
    fn the_choice_and_the_outcome_cross_the_bridge_as_the_strings_the_ui_sends() {
        // `src/lib/api.ts` writes these literals by hand; a serde rename on
        // this side would fail silently at the bridge rather than at compile
        // time, leaving the buttons inert.
        assert_eq!(serde_json::to_value(Keep::Parked).unwrap(), "parked");
        assert_eq!(serde_json::to_value(Keep::Current).unwrap(), "current");
        assert_eq!(serde_json::to_value(Resolved::Done).unwrap()["outcome"], "done");
        assert_eq!(serde_json::to_value(Resolved::Stale).unwrap()["outcome"], "stale");

        let raced = Resolved::Conflict { conflict_path: "nodes/character/kael.conflict-a-b.md".into() };
        let json = serde_json::to_value(&raced).unwrap();
        assert_eq!(json["outcome"], "conflict");
        assert_eq!(json["conflictPath"], "nodes/character/kael.conflict-a-b.md");
    }
}
