//! Three-way apply: fast-forward, converge, or raise a conflict.
//!
//! [`crate::atomic::guarded_write`] protects *one* folder that several people
//! write to. Replication is a different problem and its compare-and-swap does
//! not reach it: with a replica on each machine, two people both write
//! successfully to their own copy, neither CAS fails, and the two folders simply
//! disagree. Nothing on either side can tell that apart from "one of us edited
//! it" by looking at two hashes, because two hashes only ever say *different*.
//!
//! What makes the difference is a third hash: the bytes this project and one
//! peer last agreed on, kept per (peer, node) in `sync_state` (#78). With it,
//! four cases separate cleanly, and they are the whole of this module:
//!
//! | local vs base | remote vs base | outcome |
//! | --- | --- | --- |
//! | same | changed | fast-forward — write theirs, move the base |
//! | changed | same | send ours, nothing to apply |
//! | changed | changed, same bytes | converged — just move the base |
//! | changed | changed, different bytes | **conflict** |
//!
//! ## The one invariant
//!
//! **A remote peer never overwrites a local file that this project changed.**
//! Fast-forward is the only branch that writes to a node file, and it is
//! reachable only when our bytes are *exactly* the bytes we last agreed on —
//! which means the write cannot destroy anything that is not also on the peer's
//! machine. Everything else either writes nothing or parks the incoming version
//! beside ours as a `.conflict-` sibling.
//!
//! The corollary is [`base_hash`] returning `None`. That is the ordinary
//! never-synced state, and it is read as **concurrent** — both sides changed —
//! rather than as "same". Read the other way it would fast-forward over an edit
//! nobody ever agreed to, silently, on the first contact with a peer, which is
//! the single worst thing this crate could do. Read this way the cost of being
//! wrong is a conflict card that nobody needed. Those two are not close, and no
//! optimisation may ever trade the second for the first.
//!
//! ## We do not merge prose
//!
//! Same argument as [`crate::conflict`]: three sentences two people rewrote in
//! different directions have no correct interleaving, and a machine guessing at
//! one produces text neither of them wrote. A conflict here lands as a sibling
//! and the existing card resolves it — the *incoming* version is the one parked,
//! named after the peer that sent it, so a folder full of siblings still reads
//! as "here is what each machine thought".
//!
//! ## What this module deliberately does not do
//!
//! - **It does not delete.** A node that one side has and the other does not,
//!   where the two once agreed, is somebody's deletion; M3 has no tombstones, so
//!   [`Decision::Deleted`] is a no-op and the node is left on both machines. A
//!   replicated delete driven by absence would turn a half-mounted share into a
//!   world-wide erase, and that is not a bug anyone gets to make once.
//! - **It does not send.** [`Decision::SendOurs`] is reported and nothing else;
//!   this crate has no transport and must not gain one. `wobu-sync` pushes, and
//!   the base moves only when the peer says it arrived — see
//!   [`Project::record_agreed`].
//! - **It does not emit.** A batch that changed the folder says so
//!   ([`ApplyReport::changed_the_folder`]) and the shell emits `world:changed`.
//!   Nothing in `wobu-store` may reach a Tauri `AppHandle`.
//!
//! [`base_hash`]: crate::Index::base_hash
//! [`Project::record_agreed`]: crate::Project::record_agreed

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wobu_core::Id;

use crate::atomic::{self, WriteOutcome};
use crate::conflict;
use crate::error::{Error, Result};
use crate::index::Index;
use crate::markdown;
use crate::paths;

/* ── the compare ──────────────────────────────────────────────────────────── */

/// What comparing one node against one peer says to do.
///
/// A plain enum over three hashes and nothing else — no paths, no IO, no
/// `Result`. That is on purpose: this is the decision the whole milestone turns
/// on, so it is a total function of its arguments and every row of the table can
/// be asserted in a unit test without a filesystem anywhere near it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    /// All three hashes agree, or neither side has the node. Nothing to do and
    /// no base to move — the row is already right.
    InStep,
    /// We are still at the base and they have moved on. Their version is safe to
    /// write, because ours holds nothing theirs does not.
    FastForward,
    /// We have moved on and they are still at the base. They need our bytes;
    /// nothing arrives here.
    SendOurs,
    /// Both moved, to the same bytes. Two people typed the same paragraph, or
    /// one of them applied a version they already had. Nothing is written and
    /// the base moves to what both now hold.
    Converged,
    /// Both moved, differently. The incoming version is parked beside ours and a
    /// human picks. **The base does not move** — see [`apply`].
    Conflict,
    /// One side agreed on this node and no longer has it: a deletion. Not
    /// replicated in M3, and not guessed at either — nothing is written, nothing
    /// is removed, and the base is left exactly where it was.
    Deleted,
}

/// Compare one node's three hashes.
///
/// `local` and `remote` are `None` when that side does not have the node at all,
/// which is how a creation and a deletion both arrive. `base` is `None` when
/// this project and this peer have never agreed on this node — the state every
/// pair starts in, and the one that must read as concurrent.
///
/// Every hash here is the full BLAKE3 hex [`crate::atomic::Stamp`] produces, not
/// the truncated `source_version` in `index.rs`. Those two must never be
/// swapped: a `source_version` is deliberately blind to names and summaries, so
/// a sync built on one would let a rename vanish between two machines without
/// either of them noticing.
pub fn decide(local: Option<&str>, remote: Option<&str>, base: Option<&str>) -> Decision {
    match (local, remote, base) {
        // Neither machine has it. There is nothing to compare and nothing to do;
        // a leftover base row for a node both sides deleted is harmless, and
        // clearing it here would be this module removing sync state on the
        // strength of a directory listing.
        (None, None, _) => Decision::InStep,

        // One side has it and the two once agreed on it, so the other side did
        // not "never have it" — it lost it. See `Decision::Deleted`.
        (None, Some(_), Some(_)) | (Some(_), None, Some(_)) => Decision::Deleted,

        // A node we have never heard of, from a peer we never agreed anything
        // with about it: they made it. Writing it cannot destroy local text
        // because there is no local file — and if there *is* a file at the path
        // it would land on, `apply` sees its hash as `local` and this is a
        // `Conflict` instead.
        (None, Some(_), None) => Decision::FastForward,
        (Some(_), None, None) => Decision::SendOurs,

        // No base: both sides are, as far as anything here knows, changed. The
        // whole safety argument of the module is in this arm. Identical bytes
        // are still not a conflict — nobody's text is at risk when both machines
        // hold the same paragraph — so the base simply catches up.
        (Some(l), Some(r), None) => {
            if l == r {
                Decision::Converged
            } else {
                Decision::Conflict
            }
        }

        (Some(l), Some(r), Some(b)) => match (l == b, r == b) {
            (true, true) => Decision::InStep,
            (true, false) => Decision::FastForward,
            (false, true) => Decision::SendOurs,
            (false, false) if l == r => Decision::Converged,
            (false, false) => Decision::Conflict,
        },
    }
}

/* ── what crosses the wire ────────────────────────────────────────────────── */

/// One node as a peer sends it: the whole Markdown file, and the two things
/// needed to place it.
///
/// The bytes travel whole rather than as a patch. A patch would need a common
/// ancestor's *contents* to apply against, and this crate keeps only the
/// ancestor's hash — deliberately, because storing every peer's last-agreed body
/// would double the size of every project on disk to save bandwidth on files
/// that are a few kilobytes each.
///
/// There is no `hash` field and that is not an oversight. A hash the sender
/// declares is a hash the sender can get wrong, and every decision below would
/// then be made about bytes nobody checked. It is computed here from `text`, so
/// there is nothing to disagree with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Incoming {
    /// Which node this claims to be. Checked against the id inside `text`, and
    /// a mismatch is refused rather than resolved in either direction — a peer
    /// that can name one node and send another's bytes can overwrite any file
    /// in the folder.
    pub node_id: Id,
    /// The peer's filename stem for this node.
    ///
    /// Needed only for a node we have never seen, because a node file's slug
    /// lives in its *name* and not in its frontmatter. For a node we already
    /// hold, our own path wins and this is ignored: where a file sits is local
    /// bookkeeping, and letting a peer relocate our files is a much larger
    /// permission than letting it edit them.
    ///
    /// Validated with [`wobu_core::is_valid_slug`] before it is ever joined onto
    /// a path. That check is what makes a `../../` impossible, so it must stay
    /// on the near side of the join.
    pub slug: String,
    pub text: String,
}

/// One node packaged the way a peer will receive it, with the hash a manifest
/// carries.
///
/// The send half of the same seam, here rather than in `wobu-sync` so that both
/// directions agree on what a node payload is. Nothing in this crate transmits
/// it — see the module doc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outgoing {
    pub node_id: Id,
    pub slug: String,
    pub text: String,
    /// Full BLAKE3 hex of `text`, as the folder holds it right now.
    pub hash: String,
}

impl From<Outgoing> for Incoming {
    fn from(out: Outgoing) -> Incoming {
        Incoming { node_id: out.node_id, slug: out.slug, text: out.text }
    }
}

/* ── what came of it ──────────────────────────────────────────────────────── */

/// Why an incoming node was not applied at all.
///
/// Refusals are per-node and never abort the batch. A peer sending one mangled
/// file must not be able to stop the other two hundred good ones landing — that
/// would turn a corrupt byte on one machine into a sync that never completes for
/// anybody.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "refusal", rename_all = "camelCase")]
pub enum Refused {
    /// The bytes do not parse as a node file. Half-transferred, or written by a
    /// build that renders something this one cannot read.
    #[serde(rename_all = "camelCase")]
    Unreadable { reason: String },
    /// It parses, and it is a different node than the one it was announced as.
    #[serde(rename_all = "camelCase")]
    WrongNode { contained: Id },
    /// A node we have never seen, whose filename stem is not a slug this crate
    /// will put on a path. The only defence against a peer naming a file
    /// `../../../.ssh/authorized_keys`.
    #[serde(rename_all = "camelCase")]
    UnusableSlug { slug: String },
}

/// What happened to one node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "applied", rename_all = "camelCase")]
pub enum Applied {
    /// Nothing to do; the folder was already in step with the peer.
    InStep,
    /// Their version is now the file on disk and the base moved with it.
    #[serde(rename_all = "camelCase")]
    FastForwarded { rel_path: String },
    /// Both sides already held these bytes. Nothing was written; the base moved.
    Converged,
    /// Ours is ahead. Nothing was written and **the base did not move** — it
    /// moves when the peer acknowledges, not when we decide to send.
    SendOurs,
    /// Their version is parked beside ours and the node file is untouched.
    #[serde(rename_all = "camelCase")]
    Conflicted { conflict_path: String },
    /// Their version was already parked beside ours from an earlier round, byte
    /// for byte. Nothing was written — see [`apply`] for why this exists.
    #[serde(rename_all = "camelCase")]
    AlreadyParked { conflict_path: String },
    /// One side deleted it. Nothing was written and nothing was removed.
    Deleted,
    /// The payload was not usable. Nothing was written.
    Refused(Refused),
}

/// What a batch did, in the order it was handed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    /// The peer whose bytes these were, as the 64-hex `EndpointId`.
    pub peer_id: String,
    pub outcomes: Vec<(Id, Applied)>,
}

impl ApplyReport {
    /// Whether anything in the project folder moved.
    ///
    /// This is the whole of the `world:changed` seam. The shell emits once per
    /// batch when this is true, rather than this crate emitting per node —
    /// `wobu-store` has no `AppHandle` and must not grow one, and a
    /// two-hundred-node sync firing two hundred refetches would be worse than
    /// not refetching at all.
    pub fn changed_the_folder(&self) -> bool {
        self.outcomes.iter().any(|(_, a)| {
            matches!(a, Applied::FastForwarded { .. } | Applied::Conflicted { .. })
        })
    }

    /// The nodes the peer is behind on, for the caller to push.
    pub fn to_send(&self) -> Vec<Id> {
        self.outcomes
            .iter()
            .filter(|(_, a)| matches!(a, Applied::SendOurs))
            .map(|(id, _)| *id)
            .collect()
    }

    /// Siblings this batch created, project-relative. `AlreadyParked` is
    /// deliberately not in here: nothing new landed, so raising the card again
    /// would be announcing a decision the user has already been asked for.
    pub fn parked(&self) -> Vec<&str> {
        self.outcomes
            .iter()
            .filter_map(|(_, a)| match a {
                Applied::Conflicted { conflict_path } => Some(conflict_path.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn refusals(&self) -> Vec<(Id, &Refused)> {
        self.outcomes
            .iter()
            .filter_map(|(id, a)| match a {
                Applied::Refused(why) => Some((*id, why)),
                _ => None,
            })
            .collect()
    }
}

/* ── the batch ────────────────────────────────────────────────────────────── */

/// Fold a peer's nodes into the folder.
///
/// Nodes are processed in the order given and each one is decided *afresh
/// against the disk*, not against whatever a manifest exchange concluded a
/// moment ago. That matters: the user is still typing while a sync runs, so the
/// local hash a [`plan`] computed is already historical by the time bytes
/// arrive. Re-reading is one `stat` and one read of a file we were about to
/// write anyway.
///
/// ## Where the bases are moved
///
/// In one [`Index::record_bases`] transaction, after the last node, and only if
/// every node got that far. An error part-way through returns `Err` and records
/// **nothing** — including for the nodes that already landed on disk.
///
/// That is the safe direction and it is worth being explicit about why. A base
/// that is behind costs a re-compare: the node that did land now reads
/// local == remote, which is `Converged`, and the base catches up on the next
/// round with nothing written. A base that ran *ahead* of the folder would claim
/// agreement on bytes this machine does not have, and the next round would
/// fast-forward on that claim — a write over a file nobody agreed to. Half a
/// base set is worse than none, so there is no half.
///
/// ## Why an already-parked sibling is not parked twice
///
/// A conflict does not move the base — neither side agreed anything, so there is
/// nothing to record — which means the same disagreement is rediscovered on
/// every round until a human resolves it. Left alone that grows one sibling per
/// sync until the folder is unusable, and the user cannot tell the copies apart
/// because they are identical. So an incoming version whose bytes are already
/// sitting beside the target is reported and not written again. The check reads
/// the siblings of one file, in the conflict branch only, which is the rare one.
///
/// This is also what makes losing the index survivable: a schema bump drops
/// every base, the next sync reads the whole world as concurrent, and without
/// this it would park a second copy of every divergent file at once.
pub fn apply(
    root: &Path,
    index: &Index,
    peer_id: &str,
    incoming: &[Incoming],
) -> Result<ApplyReport> {
    let bases = index.bases_for_peer(peer_id)?;
    // Their alias, not ours. Every sibling this function writes holds *their*
    // bytes, so stamping it with this installation's name would attribute a
    // paragraph on the card to the person who did not write it.
    let sender = alias_of(peer_id);

    let mut outcomes: Vec<(Id, Applied)> = Vec::with_capacity(incoming.len());
    let mut settled: Vec<(Id, String)> = Vec::new();

    for item in incoming {
        let id = item.node_id;
        let their_hash = atomic::hash_bytes(item.text.as_bytes());
        let base = bases.get(&id).map(String::as_str);

        // Parsed before anything is placed, and the id checked, because the
        // whole of the trust boundary is here: after this point a peer's bytes
        // are written to a path derived from a `NodeKind` this build defines and
        // a slug this build validated.
        let announced = paths::from_rel_string(Path::new(peer_id), &format!("{id}.md"));
        let mut node = match markdown::from_markdown(&item.text, &announced) {
            Ok(node) => node,
            Err(e) => {
                outcomes.push((id, Applied::Refused(Refused::Unreadable { reason: e.to_string() })));
                continue;
            }
        };
        if node.id != id {
            outcomes.push((id, Applied::Refused(Refused::WrongNode { contained: node.id })));
            continue;
        }

        let rel = match index.rel_path_of(id)? {
            Some(rel) => rel,
            None => {
                if !wobu_core::is_valid_slug(&item.slug) {
                    let slug = item.slug.clone();
                    outcomes.push((id, Applied::Refused(Refused::UnusableSlug { slug })));
                    continue;
                }
                format!("nodes/{}/{}.md", node.kind.dir(), item.slug)
            }
        };
        // `from_markdown` takes the slug from the filename, and the filename it
        // was handed above is a placeholder. Overwriting it with the stem of the
        // path we are actually going to use is what keeps the index row and the
        // file on disk from disagreeing about what this node is called.
        node.slug = stem_of(&rel);
        let path = paths::from_rel_string(root, &rel);

        let local = atomic::read_stamped(&path)?;
        let local_hash = local.as_ref().map(|(_, s)| s.hash.as_str());

        match decide(local_hash, Some(&their_hash), base) {
            Decision::InStep => outcomes.push((id, Applied::InStep)),
            Decision::SendOurs => outcomes.push((id, Applied::SendOurs)),
            Decision::Deleted => outcomes.push((id, Applied::Deleted)),
            Decision::Converged => {
                settled.push((id, their_hash));
                outcomes.push((id, Applied::Converged));
            }
            Decision::FastForward => {
                // Through `guarded_write` rather than a plain write. The read
                // above closed nothing: a local save can land in the
                // microseconds between it and the rename, and on a share so can
                // another machine's. If it does, the CAS fails and *their*
                // version is the one parked — the local file that won is left
                // exactly as it is, which is the same promise `save_node` makes.
                let expected = local.map(|(_, stamp)| stamp);
                match atomic::guarded_write(root, &path, &item.text, expected.as_ref(), &sender)? {
                    WriteOutcome::Written(stamp) => {
                        index.upsert_node(&node, &rel, &stamp)?;
                        index.clear_corrupt(&rel)?;
                        settled.push((id, their_hash));
                        outcomes.push((id, Applied::FastForwarded { rel_path: rel }));
                    }
                    WriteOutcome::Conflict { conflict_path, .. } => {
                        // Lost the race. The base stays where it was, for the
                        // same reason it does in the `Conflict` arm below:
                        // nothing was agreed, and the version that won is one
                        // this project has never shown the peer.
                        let conflict_path = relative_to(root, &conflict_path);
                        outcomes.push((id, Applied::Conflicted { conflict_path }));
                    }
                }
            }
            Decision::Conflict => match already_parked(&path, &their_hash)? {
                Some(existing) => {
                    let conflict_path = relative_to(root, &existing);
                    outcomes.push((id, Applied::AlreadyParked { conflict_path }));
                }
                None => {
                    let (parked, _) = atomic::park_conflict(root, &path, &item.text, &sender)?;
                    let conflict_path = relative_to(root, &parked);
                    outcomes.push((id, Applied::Conflicted { conflict_path }));
                }
            },
        }
    }

    index.record_bases(peer_id, &settled)?;
    Ok(ApplyReport { peer_id: peer_id.to_string(), outcomes })
}

/// A sibling beside `target` already holding exactly these bytes.
///
/// Content, not filename: the name carries a timestamp and a collision suffix,
/// so two parks of the same text are guaranteed to have different names and
/// comparing names would find nothing. Siblings of *other* files in the same
/// directory are skipped by asking [`conflict::parse`] what each one sits beside,
/// rather than by matching prefixes — `kael.md` and `kael-vantris.md` share one.
fn already_parked(target: &Path, hash: &str) -> Result<Option<PathBuf>> {
    let (Some(dir), Some(name)) = (target.parent(), target.file_name().and_then(|n| n.to_str()))
    else {
        return Ok(None);
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // The node file's own directory not existing means there are no
        // siblings, which is the answer, not a failure.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(dir, e)),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(parsed) = path.file_name().and_then(|n| n.to_str()).and_then(conflict::parse)
        else {
            continue;
        };
        if parsed.target_file_name() != name {
            continue;
        }
        // A sibling that vanished between the listing and the read is a
        // collaborator resolving the same card. It is not there, so it is not a
        // match, and the sync carries on.
        if let Some((_, stamp)) = atomic::read_stamped(&path)?
            && stamp.hash == hash
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/* ── the manifest diff ────────────────────────────────────────────────────── */

/// What a peer's manifest implies, without touching anything.
///
/// #79 exchanges `(node_id, hash)` pairs before any bytes move, so that a sync
/// over a slow link transfers only the files that actually differ. This is that
/// comparison, and it is read-only on purpose: a plan is a *proposal*, and the
/// project folder is not entitled to change because two machines exchanged a
/// list.
///
/// Local hashes come from the index rather than from the disk. That is a
/// deliberate staleness: it costs one query instead of re-reading every node
/// file over SMB, and it is safe because nothing acts on it — [`apply`] decides
/// again against the disk when the bytes arrive. The worst a stale plan can do
/// is ask for a file it did not need.
pub fn plan(index: &Index, peer_id: &str, remote: &[(Id, String)]) -> Result<Plan> {
    let bases = index.bases_for_peer(peer_id)?;
    let ours = index.node_hashes()?;
    let theirs: HashMap<Id, &str> = remote.iter().map(|(id, h)| (*id, h.as_str())).collect();

    // Sorted and deduplicated so a plan is a function of the two manifests and
    // not of `HashMap` iteration order. A work list that shuffles between runs
    // is one nobody can diff against the last one while debugging a sync.
    let mut ids: Vec<Id> = ours.keys().copied().chain(theirs.keys().copied()).collect();
    ids.sort_unstable();
    ids.dedup();

    let mut plan = Plan::default();
    for id in ids {
        let local = ours.get(&id).map(String::as_str);
        let remote = theirs.get(&id).copied();
        match decide(local, remote, bases.get(&id).map(String::as_str)) {
            // Both need their bytes: one to write, one to park.
            Decision::FastForward | Decision::Conflict => plan.wanted.push(id),
            Decision::SendOurs => plan.send.push(id),
            Decision::Converged => {
                // `remote` is `Some` in every branch `Converged` can be reached
                // from — it is the arm where both sides hold identical bytes.
                if let Some(hash) = remote {
                    plan.settled.push((id, hash.to_string()));
                }
            }
            Decision::Deleted => plan.skipped.push(id),
            Decision::InStep => {}
        }
    }
    Ok(plan)
}

/// The work a manifest exchange turned up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// Ask the peer for these bodies, then hand them to [`apply`].
    pub wanted: Vec<Id>,
    /// Push these to the peer. Their bases move on acknowledgement, through
    /// [`crate::Project::record_agreed`], and not before.
    pub send: Vec<Id>,
    /// Already identical on both machines. Safe to record as agreed the moment
    /// the exchange completes; no bytes need to move either way.
    pub settled: Vec<(Id, String)>,
    /// One side deleted these. Left alone — M3 does not replicate deletions.
    pub skipped: Vec<Id>,
}

impl Plan {
    /// Whether anything at all has to happen. An exchange between two peers in
    /// step should cost one comparison and no transfers.
    pub fn is_empty(&self) -> bool {
        self.wanted.is_empty() && self.send.is_empty() && self.settled.is_empty()
    }
}

/* ── small helpers ────────────────────────────────────────────────────────── */

/// The filename an alias goes into, for the peer on the other end.
///
/// [`crate::peer::alias`] is only ever about *us*; this is the same derivation
/// applied to somebody else's id. It is a pure function of the hex, exactly as
/// [`wobu_core::peer::alias`] promises, so a sibling written here and a sibling
/// written on the sender's own machine carry the same name and a person reading
/// the folder can match them up.
///
/// An id that is not 64 hex characters cannot come out of iroh, but this takes a
/// `&str` and so must survive one. It is hashed into the same shape rather than
/// rejected: the sibling is somebody's only copy of a paragraph either way, and
/// a name that failed to be a name would make the file unresolvable from inside
/// the app. Note that it is a *name*, never a decision — twenty-eight bits is
/// grindable, and nothing here may ever compare aliases to decide anything.
fn alias_of(peer_id: &str) -> String {
    match endpoint_bytes(peer_id) {
        Some(bytes) => wobu_core::peer::alias(&bytes),
        None => wobu_core::peer::alias(blake3::hash(peer_id.as_bytes()).as_bytes()),
    }
}

/// Sixty-four hex characters back into the thirty-two bytes iroh printed.
///
/// Hand-rolled rather than pulled in as a dependency, because this is the only
/// hex in the crate and the alternative is a supply-chain edge for eight lines.
fn endpoint_bytes(peer_id: &str) -> Option<[u8; 32]> {
    if peer_id.len() != 64 || !peer_id.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (byte, pair) in out.iter_mut().zip(peer_id.as_bytes().chunks_exact(2)) {
        let hi = char::from(pair[0]).to_digit(16)?;
        let lo = char::from(pair[1]).to_digit(16)?;
        *byte = (hi * 16 + lo) as u8;
    }
    Some(out)
}

/// `nodes/character/kael-vantris.md` → `kael-vantris`.
fn stem_of(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name).to_string()
}

fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(paths::to_rel_string)
        .unwrap_or_else(|_| paths::to_rel_string(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three hashes that are definitely different from each other. Real BLAKE3
    /// hex, because `decide` compares strings and a test using `"a"`/`"b"` would
    /// pass just as well against a comparison that had been broken into
    /// something length-dependent.
    fn h(bytes: &[u8]) -> String {
        atomic::hash_bytes(bytes)
    }

    /* ── the truth table, row by row ──────────────────────────────────── */

    #[test]
    fn we_are_at_the_base_and_they_moved_so_we_take_theirs() {
        let (base, theirs) = (h(b"v1"), h(b"v2"));
        assert_eq!(decide(Some(&base), Some(&theirs), Some(&base)), Decision::FastForward);
    }

    #[test]
    fn we_moved_and_they_are_at_the_base_so_they_need_ours() {
        let (base, ours) = (h(b"v1"), h(b"v2"));
        assert_eq!(decide(Some(&ours), Some(&base), Some(&base)), Decision::SendOurs);
    }

    #[test]
    fn both_moved_to_the_same_bytes_and_nobody_has_to_choose() {
        // Two people typing the same paragraph, or one of them applying a
        // version they already had from a third machine. Nothing is at risk, so
        // a conflict card here would be pure noise — and noise is expensive,
        // because it teaches people to dismiss the card that one day matters.
        let (base, agreed) = (h(b"v1"), h(b"v2"));
        assert_eq!(decide(Some(&agreed), Some(&agreed), Some(&base)), Decision::Converged);
    }

    #[test]
    fn both_moved_differently_and_that_is_the_only_conflict() {
        let (base, ours, theirs) = (h(b"v1"), h(b"mine"), h(b"yours"));
        assert_eq!(decide(Some(&ours), Some(&theirs), Some(&base)), Decision::Conflict);
    }

    #[test]
    fn all_three_agreeing_is_not_work() {
        let same = h(b"v1");
        assert_eq!(decide(Some(&same), Some(&same), Some(&same)), Decision::InStep);
    }

    /* ── the invariant ───────────────────────────────────────────────── */

    #[test]
    fn no_base_is_concurrent_and_never_a_fast_forward() {
        // The single most important assertion in this crate. `None` is the
        // ordinary state for a peer we have never synced this node with, and
        // reading it as "same" would fast-forward their file over ours on first
        // contact — silently, before anybody had a chance to look. The cost of
        // being wrong the other way is a conflict card nobody needed.
        let (ours, theirs) = (h(b"mine"), h(b"yours"));
        assert_eq!(decide(Some(&ours), Some(&theirs), None), Decision::Conflict);
        // And in the other direction, so this cannot pass by always saying
        // conflict when the base is missing.
        assert_ne!(decide(Some(&ours), Some(&theirs), None), Decision::FastForward);
        assert_ne!(decide(Some(&ours), Some(&theirs), None), Decision::SendOurs);
    }

    #[test]
    fn no_base_with_identical_bytes_still_only_moves_the_base() {
        // The other half: never-synced must not mean never-agreeing. Two
        // machines holding the same file have nothing to transfer and nothing to
        // resolve, and the very first exchange between two peers with a shared
        // history is almost entirely this case.
        let same = h(b"identical");
        assert_eq!(decide(Some(&same), Some(&same), None), Decision::Converged);
    }

    #[test]
    fn a_base_that_matches_neither_side_is_a_conflict_not_a_guess() {
        // Both machines moved on from an agreement neither still holds. There is
        // no third version to prefer, so there is nothing to do but ask.
        let (base, ours, theirs) = (h(b"old"), h(b"a"), h(b"b"));
        assert_eq!(decide(Some(&ours), Some(&theirs), Some(&base)), Decision::Conflict);
    }

    /* ── nodes one side does not have ────────────────────────────────── */

    #[test]
    fn a_node_only_they_have_and_never_agreed_on_is_theirs_to_give() {
        // How a node created on another machine arrives. Safe because there is
        // no local file — `apply` reads the destination and turns this into a
        // conflict the moment there is something there to lose.
        let theirs = h(b"new");
        assert_eq!(decide(None, Some(&theirs), None), Decision::FastForward);
    }

    #[test]
    fn a_node_only_we_have_and_never_agreed_on_is_ours_to_send() {
        let ours = h(b"new");
        assert_eq!(decide(Some(&ours), None, None), Decision::SendOurs);
    }

    #[test]
    fn a_node_that_went_missing_after_an_agreement_is_left_completely_alone() {
        // A deletion, on whichever side. M3 has no tombstones, so the two
        // available guesses are "resurrect it" and "delete ours" — and the
        // second one, driven by an absence, turns a half-mounted share into a
        // world-wide erase. Neither is guessed.
        let agreed = h(b"v1");
        assert_eq!(decide(None, Some(&agreed), Some(&agreed)), Decision::Deleted);
        assert_eq!(decide(Some(&agreed), None, Some(&agreed)), Decision::Deleted);
        // Including when the surviving side also edited it, which is the case a
        // "the other side deleted it, so delete ours" rule loses outright.
        let edited = h(b"v2");
        assert_eq!(decide(Some(&edited), None, Some(&agreed)), Decision::Deleted);
    }

    #[test]
    fn a_node_neither_side_has_is_nothing_at_all() {
        assert_eq!(decide(None, None, None), Decision::InStep);
        assert_eq!(decide(None, None, Some(&h(b"v1"))), Decision::InStep);
    }

    #[test]
    fn decide_never_writes_and_never_fails() {
        // Restating the shape as an assertion, because it is the reason every
        // row above can be tested without a filesystem: the decision is a total
        // function of three optional strings. Anything that made it return a
        // `Result`, or take a path, would move the most important logic in M3
        // somewhere it can only be tested through IO.
        for local in [None, Some("a"), Some("b")] {
            for remote in [None, Some("a"), Some("b")] {
                for base in [None, Some("a"), Some("b")] {
                    let _: Decision = decide(local, remote, base);
                }
            }
        }
    }

    /* ── naming the sender ───────────────────────────────────────────── */

    #[test]
    fn a_peers_alias_is_the_one_its_own_machine_would_write() {
        // There is no table mapping ids to names — that is #76's whole argument
        // — so a sibling named here has to carry the same name the sender's own
        // `guarded_write` would have put on it. That is only true if this is the
        // same pure function over the same bytes.
        let mut id = [0u8; 32];
        id[..4].copy_from_slice(&[0x4f, 0x1a, 0x00, 0x1d]);
        let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(alias_of(&hex), wobu_core::peer::alias(&id));
        assert_eq!(alias_of(&hex), "amber-heron-4f1a");
    }

    #[test]
    fn an_alias_is_always_a_filename_the_conflict_parser_reads_back() {
        // The contract with `atomic::conflict_sibling` and `conflict::parse`. A
        // peer id is a public key and there is no such thing as an unusual one,
        // so every alias this can produce has to be a slug — including the ones
        // from the fallback, which is reached by a hex string that is not one.
        for peer_id in [
            "0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "not hex at all",
            "",
            // Right length, wrong alphabet.
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        ] {
            let alias = alias_of(peer_id);
            assert!(wobu_core::is_valid_slug(&alias), "{peer_id} produced {alias}");
            let name = format!("kael-vantris.conflict-{alias}-20260731T142211Z.md");
            let parsed = conflict::parse(&name).expect("a sibling");
            assert_eq!(parsed.peer.as_deref(), Some(alias.as_str()));
            assert_eq!(parsed.target_file_name(), "kael-vantris.md");
        }
    }

    #[test]
    fn an_unparseable_id_still_names_the_same_peer_every_time() {
        // A fallback that wandered would scatter one machine's siblings across
        // several names, which is the `$USER` bug inverted and just as
        // unreadable.
        assert_eq!(alias_of("not hex at all"), alias_of("not hex at all"));
        assert_ne!(alias_of("not hex at all"), alias_of("also not hex"));
    }

    #[test]
    fn hex_decoding_is_exact_about_length_and_alphabet() {
        assert_eq!(endpoint_bytes(&"00".repeat(32)), Some([0u8; 32]));
        assert_eq!(endpoint_bytes(&"ff".repeat(32)), Some([0xffu8; 32]));
        assert_eq!(endpoint_bytes(&"0".repeat(63)), None, "63 characters is not an id");
        assert_eq!(endpoint_bytes(&"0".repeat(65)), None, "65 characters is not an id");
        assert_eq!(endpoint_bytes(&"g".repeat(64)), None, "g is not hex");
        // A multi-byte character can make `len()` 64 while the string is 63
        // characters, and indexing bytes then splits it in half.
        let sneaky = format!("é{}", "0".repeat(62));
        assert_eq!(sneaky.len(), 64);
        assert_eq!(endpoint_bytes(&sneaky), None, "a non-ASCII id must not be decoded");
    }

    /* ── placing a file ──────────────────────────────────────────────── */

    #[test]
    fn a_stem_is_taken_from_the_path_we_are_going_to_use() {
        assert_eq!(stem_of("nodes/character/kael-vantris.md"), "kael-vantris");
        assert_eq!(stem_of("kael-vantris.md"), "kael-vantris");
        assert_eq!(stem_of("nodes/character/kael"), "kael");
    }
}
