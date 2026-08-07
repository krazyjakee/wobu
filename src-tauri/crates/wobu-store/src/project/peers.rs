//! What this project sends another machine, and what it accepts back.
//!
//! Accepting is deliberately conservative: an incoming node either applies
//! cleanly or is parked as a conflict beside the local copy, and the local file
//! is never overwritten in place. `record_refusal` exists so that a peer told
//! "no" learns why rather than retrying the same body forever.

use std::path::Path;

use wobu_core::{DescriptionState, Id, Node, NodeKind, kind_def};

use super::*;
use crate::apply;
use crate::atomic::{self, WriteOutcome};
use crate::conflict::{self, Conflict, Keep, Resolved};
use crate::error::{Error, Result};
use crate::markdown;
use crate::paths;
use crate::transfer::{TransferBundle, TransferOutcome};

impl Project {
    /// Apply a fully staged style/subtree transfer to this project.
    ///
    /// The complete target graph, filenames and Markdown are validated before
    /// any node is written. Asset copies happen first and are content-addressed,
    /// so an unexpected failure can leave only harmless reusable blobs before
    /// the first entity. Once node publication starts, every guarded write is
    /// accounted for in the returned report; a conflict is never flattened
    /// into an apparent all-or-nothing success.
    pub fn apply_transfer(&mut self, bundle: TransferBundle) -> Result<TransferOutcome> {
        self.apply_transfer_with(bundle, |_| {})
    }

    pub(super) fn apply_transfer_with(
        &mut self,
        bundle: TransferBundle,
        after_preflight: impl FnOnce(&mut Project),
    ) -> Result<TransferOutcome> {
        self.ensure_writable()?;
        let destination_root =
            std::fs::canonicalize(&self.root).map_err(|error| Error::io(&self.root, error))?;
        if destination_root == bundle.source_root || self.id() == bundle.source_project_id {
            return Err(Error::TransferSameProject);
        }

        let selected: std::collections::HashSet<Id> =
            bundle.nodes.iter().map(|node| node.id).collect();
        if !bundle.nodes.iter().any(|node| node.id == bundle.root_id) {
            return Err(Error::NoSuchNode(bundle.root_id.to_string()));
        }

        // Identity and slug allocation is an in-memory preflight. Singleton
        // identity is kept exactly so existing destination backlinks survive;
        // ordinary nodes receive fresh ids and collision-free paths.
        let existing = self.list_nodes()?;
        let mut taken: std::collections::HashMap<NodeKind, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        for node in &existing {
            taken.entry(node.kind).or_default().insert(node.slug.clone());
        }
        let mut identities: std::collections::HashMap<Id, Node> = std::collections::HashMap::new();
        for source in &bundle.nodes {
            if kind_def(source.kind).singleton {
                let summary = existing
                    .iter()
                    .find(|node| node.kind == source.kind)
                    .ok_or_else(|| Error::NoSuchNode(source.kind.as_str().to_string()))?;
                identities.insert(source.id, self.get_node(summary.id)?);
                continue;
            }

            let mut identity = Node::new(source.kind, &source.name)?;
            while self.index.node(identity.id)?.is_some()
                || identities.values().any(|node| node.id == identity.id)
            {
                identity.id = wobu_core::new_id();
            }
            let kind_taken = taken.entry(source.kind).or_default();
            identity.slug = wobu_core::unique_slug(&identity.slug, &|slug| {
                kind_taken.contains(slug)
                    || self
                        .root
                        .join(format!("{NODES_DIR}/{}/{}.md", source.kind.dir(), slug))
                        .exists()
            });
            kind_taken.insert(identity.slug.clone());
            identities.insert(source.id, identity);
        }

        let id_map: std::collections::HashMap<Id, Id> =
            identities.iter().map(|(source, target)| (*source, target.id)).collect();
        let mut planned = Vec::with_capacity(bundle.nodes.len());
        for source in &bundle.nodes {
            let identity = identities
                .get(&source.id)
                .ok_or_else(|| Error::NoSuchNode(source.id.to_string()))?;
            let mut target = source.clone();
            target.id = identity.id;
            target.slug = identity.slug.clone();
            target.created_at = identity.created_at;
            target.parent_id = source.parent_id.and_then(|parent| id_map.get(&parent).copied());
            target.links.retain(|link| selected.contains(&link.to_id));
            for link in &mut target.links {
                link.to_id = id_map[&link.to_id];
            }
            target.enhanced_from = None;
            target.description_state = match &target.description {
                Some(description) if !description.is_empty() => DescriptionState::Edited,
                _ => DescriptionState::None,
            };
            target.touch();
            planned.push(target);
        }

        let planned_lookup: std::collections::HashMap<Id, (NodeKind, Option<Id>)> =
            planned.iter().map(|node| (node.id, (node.kind, node.parent_id))).collect();
        for node in &planned {
            node.validate()?;
            let lookup = |id: Id| {
                planned_lookup
                    .get(&id)
                    .copied()
                    .or_else(|| self.index.kind_and_parent(id).ok().flatten())
            };
            wobu_core::validate_parent(node, node.parent_id, &lookup)?;
            // Serialisation belongs in preflight too: a value that cannot be
            // represented in frontmatter must not be discovered half-way in.
            let _ = markdown::to_markdown(node)?;
        }
        for asset in &bundle.assets {
            crate::assets::validate_import(&asset.bytes)?;
            let hash = atomic::hash_bytes(&asset.bytes);
            if crate::assets::asset_id(&hash) != Some(asset.id) {
                return Err(Error::NoSuchAsset(asset.id.to_string()));
            }
        }
        for lora in &bundle.loras {
            crate::lora::validate(&lora.bytes)?;
            if atomic::hash_bytes(&lora.bytes) != lora.hash {
                return Err(Error::InvalidLora(
                    "a staged transfer weight no longer matches its content hash".into(),
                ));
            }
        }

        // Parents publish first. This also makes a singleton root's children
        // point at its preserved destination id before their own write.
        planned.sort_by_key(|node| {
            let mut depth = 0usize;
            let mut parent = node.parent_id;
            while let Some(id) = parent {
                let Some((_, next)) = planned_lookup.get(&id) else { break };
                depth += 1;
                parent = *next;
            }
            depth
        });
        let expected_stamps: std::collections::HashMap<Id, Option<atomic::Stamp>> = planned
            .iter()
            .map(|node| {
                let stamp = if kind_def(node.kind).singleton {
                    self.index.stamp_of(node.id)
                } else {
                    Ok(None)
                }?;
                Ok((node.id, stamp))
            })
            .collect::<Result<_>>()?;

        let imported_root_id = id_map[&bundle.root_id];
        let mut outcome = TransferOutcome {
            completed: false,
            root_id: bundle.root_id,
            imported_root_id,
            planned_node_count: planned.len(),
            applied_node_ids: Vec::new(),
            pending_node_ids: planned.iter().map(|node| node.id).collect(),
            reference_count: bundle.assets.len(),
            deduped_reference_count: 0,
            lora_count: bundle.loras.len(),
            deduped_lora_count: 0,
            dropped_external_link_count: bundle.external_link_count,
            replaced_singleton: bundle.replaces_singleton,
            conflict_paths: Vec::new(),
            failure: None,
        };

        // Production passes a no-op. The seam makes the race contract
        // deterministic in a unit test: a collaborator can win after every
        // byte/path preflight but before the first guarded publication.
        after_preflight(self);

        match publish_transfer_items(&bundle.assets, |asset| {
            self.import_asset(&asset.bytes, asset.kind).map(|imported| imported.deduped)
        }) {
            Ok(deduped) => outcome.deduped_reference_count = deduped,
            Err(failure) => {
                outcome.deduped_reference_count = failure.deduped;
                outcome.failure = Some(format!(
                    "Reference {} of {} failed: {}",
                    failure.index + 1,
                    bundle.assets.len(),
                    failure.error
                ));
                return Ok(outcome);
            }
        }

        match publish_transfer_items(&bundle.loras, |lora| {
            crate::lora::publish(&self.root, &lora.hash, &lora.bytes).map(|(_, deduped)| deduped)
        }) {
            Ok(deduped) => outcome.deduped_lora_count = deduped,
            Err(failure) => {
                outcome.deduped_lora_count = failure.deduped;
                outcome.failure = Some(format!(
                    "LoRA {} of {} failed: {}",
                    failure.index + 1,
                    bundle.loras.len(),
                    failure.error
                ));
                return Ok(outcome);
            }
        }

        for node in planned {
            let expected = expected_stamps.get(&node.id).and_then(Option::as_ref);
            match self.write_node(&node, expected) {
                Ok(SaveOutcome::Saved(saved)) => {
                    outcome.applied_node_ids.push(saved.id);
                    outcome.pending_node_ids.retain(|id| *id != saved.id);
                }
                Ok(SaveOutcome::Conflict { conflict_path }) => {
                    outcome.conflict_paths.push(conflict_path.clone());
                    outcome.failure = Some(format!(
                        "A destination node changed during transfer; the incoming version was parked as {conflict_path}."
                    ));
                    return Ok(outcome);
                }
                Err(error) => {
                    outcome.failure = Some(error.to_string());
                    return Ok(outcome);
                }
            }
        }
        outcome.completed = true;
        Ok(outcome)
    }

    /// Fold a peer's version of some nodes into the folder.
    ///
    /// The one entry point that lets a remote machine change this project, and
    /// it is bounded: a node file is only ever *written* when our copy is
    /// byte-identical to what the two of us last agreed on, so the write cannot
    /// destroy anything that is not also on the peer's disk. Everything else
    /// parks beside ours. See [`crate::apply`] for the table and the argument.
    ///
    /// Refuses while the folder is unreachable, through the same check every
    /// other write takes. That is not a formality here: a write into an
    /// unmounted share's leftover mountpoint succeeds, landing on the local disk
    /// under the mount where nobody will ever see it — and a sync is the one
    /// writer that runs without a person watching, so it is the one most likely
    /// to do it a hundred times before anyone notices.
    ///
    /// The caller emits `world:changed` once per batch when
    /// [`ApplyReport::changed_the_folder`] says so. This crate cannot emit and
    /// must not learn how.
    ///
    /// [`ApplyReport::changed_the_folder`]: crate::apply::ApplyReport::changed_the_folder
    pub fn apply_from_peer(
        &mut self,
        peer_id: &str,
        incoming: &[apply::Incoming],
    ) -> Result<apply::ApplyReport> {
        self.ensure_writable()?;
        apply::apply(&self.root, &self.index, peer_id, incoming)
    }

    /// What a peer's manifest implies, without touching anything.
    ///
    /// `&self` rather than `&mut self`, and that is load-bearing rather than
    /// tidy: a plan is a proposal, and the folder is not entitled to change
    /// because two machines exchanged a list of hashes.
    pub fn plan_against_peer(&self, peer_id: &str, remote: &[(Id, String)]) -> Result<apply::Plan> {
        apply::plan(&self.index, peer_id, remote)
    }

    /// This project's half of a manifest: every node it holds, and its hash.
    ///
    /// Sorted by id so two runs against an unchanged folder produce identical
    /// bytes on the wire, which is what lets a peer skip a comparison it has
    /// already made.
    pub fn manifest(&self) -> Result<Vec<(Id, String)>> {
        let mut out: Vec<(Id, String)> = self.index.node_hashes()?.into_iter().collect();
        out.sort_unstable_by_key(|(id, _)| *id);
        Ok(out)
    }

    /// One node packaged the way a peer will receive it, straight off the disk.
    ///
    /// The bytes are read from the file rather than re-rendered from the index,
    /// so what a peer is sent is what a person would see if they opened the
    /// folder. Re-rendering would be a second definition of the file's contents
    /// and the two would eventually disagree — at which point the hash in the
    /// manifest describes one of them and the payload is the other, and the
    /// receiving end fast-forwards onto a version nobody has.
    ///
    /// `Ok(None)` means the index does not know the node, or its file is not
    /// there. Neither is an error: a node deleted between a manifest and a
    /// request is ordinary, and there is nothing to send.
    pub fn outgoing(&self, id: Id) -> Result<Option<apply::Outgoing>> {
        let Some(rel) = self.index.rel_path_of(id)? else { return Ok(None) };
        let path = paths::from_rel_string(&self.root, &rel);
        let Some((text, stamp)) = atomic::read_stamped(&path)? else { return Ok(None) };
        let slug = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Some(apply::Outgoing { node_id: id, slug, text, hash: stamp.hash }))
    }

    /// Record that this project and a peer now hold the same bytes for these
    /// nodes.
    ///
    /// **Only ever called for an acknowledgement, never for a send.** A base is
    /// a claim that a specific machine holds specific bytes, and the next sync
    /// will fast-forward on that claim without asking anybody. Moving it when we
    /// put a node on the wire, rather than when the peer said it landed, would
    /// make a dropped connection into a base that describes a file the peer
    /// never received — and the edit they make next would then read as a
    /// one-sided change and overwrite ours.
    ///
    /// One transaction, so a crash leaves the bases as of the end of a sync
    /// rather than some prefix of them.
    pub fn record_agreed(&self, peer_id: &str, agreed: &[(Id, String)]) -> Result<()> {
        self.index.record_bases(peer_id, agreed)
    }

    /// Forget everything agreed *and everything refused* with a peer: a share
    /// revoked, a ticket rotated, an identity that will not be dialled again.
    ///
    /// Cheap to be wrong about in one direction only. Forgetting too much costs
    /// a re-compare and some conflict cards; forgetting too little leaves a base
    /// attributed to a machine nobody has any reason to still trust, or a
    /// refusal that would quietly withhold a version from a machine that has
    /// since been re-admitted. Reach for it whenever unsure.
    pub fn forget_peer(&self, peer_id: &str) -> Result<()> {
        self.index.forget_peer(peer_id)
    }

    // ── conflicts ────────────────────────────────────────────────────────

    /// Every unresolved conflict sibling in the folder, newest first.
    ///
    /// Recomputed from the directory rather than cached in the index, and that
    /// is deliberate. A sibling can arrive from a *different machine* — a
    /// collaborator's Wobu parking their loser onto the share — so there is no
    /// event on this side that could keep a cached list honest. The walk is one
    /// directory listing, which `docs/07-file-shares.md` establishes as the
    /// cheap half; the reads are two files per conflict, and a folder with no
    /// conflicts, which is almost all of them, pays nothing at all.
    pub fn conflicts(&self) -> Result<Vec<Conflict>> {
        let mut out = Vec::new();

        for (rel, path) in self.conflict_files() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(conflict::parse)
            else {
                continue;
            };
            // Unreadable bytes rather than a missing file: a sibling half-copied
            // by a sync client is exactly the thing not to drop off the list.
            let Some((parked, _)) = atomic::read_stamped(&path)? else { continue };

            let target = path.with_file_name(name.target_file_name());
            let target_rel = target
                .strip_prefix(&self.root)
                .map(paths::to_rel_string)
                .unwrap_or_else(|_| paths::to_rel_string(&target));
            // A node file that has since been deleted leaves the sibling as the
            // only surviving copy, so it is listed with an empty other side
            // rather than hidden.
            let (current, current_hash) = match atomic::read_stamped(&target)? {
                Some((text, stamp)) => (text, stamp.hash),
                None => (String::new(), String::new()),
            };
            let node = self.index.node_at_rel_path(&target_rel)?;

            out.push(Conflict {
                mine: name.peer.as_deref() == Some(self.peer.as_str()),
                rel_path: rel,
                node_rel_path: target_rel,
                node_id: node.as_ref().map(|(id, _)| *id),
                node_name: node.map(|(_, name)| name),
                user: name.peer,
                saved_at: name.saved_at,
                parked,
                current,
                current_hash,
            });
        }

        // Newest first, so the card a user is most likely to be looking for is
        // the one at the top. Unlabelled siblings sort last rather than first —
        // `None` would otherwise win a descending sort and put the ones we know
        // least about in front.
        out.sort_by(|a, b| match (b.saved_at, a.saved_at) {
            (Some(b), Some(a)) => b.cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => a.rel_path.cmp(&b.rel_path),
        });
        Ok(out)
    }

    /// Carry out the user's decision about one conflict sibling.
    ///
    /// **This is the only code path in Wobu that deletes a conflict sibling**,
    /// and it is reachable only from a button. Nothing on a timer, on a scan or
    /// on a rebuild may ever grow a second one — a sibling is by construction
    /// the last surviving copy of a paragraph somebody typed.
    ///
    /// `expected_hash` is the hash of the node file as the card rendered it. A
    /// mismatch means a third writer moved the file while the user was reading
    /// the diff, so the question they answered is not the question on disk:
    /// [`Resolved::Stale`] comes back and *neither* file is touched. Without
    /// that check, "keep theirs" would quietly throw away the parked version in
    /// favour of text the user never saw, which is the same silent loss the
    /// whole conflict machinery exists to prevent.
    ///
    /// [`Keep::Current`] additionally writes the refusal down, so that the next
    /// sync round does not park the same bytes again and re-ask a question that
    /// has been answered (#89). [`Keep::Parked`] needs no such bookkeeping and
    /// deliberately does none: taking their version makes their bytes the local
    /// file, so the next compare sees `local == remote` and reads `Converged`,
    /// which is not a conflict and never consults a refusal. A refusal can only
    /// ever suppress a `Conflict`, and a `Conflict` requires the two sides to
    /// differ — so any row left over from an earlier decision about this node
    /// simply stops matching, with nothing to clear. See `record_refusal` below
    /// for how a sibling's alias is turned into the peer id a refusal is keyed
    /// on, and for the two cases where it honestly cannot be.
    pub fn resolve_conflict(
        &mut self,
        rel_path: &str,
        keep: Keep,
        expected_hash: &str,
    ) -> Result<Resolved> {
        self.ensure_writable()?;

        let sibling = paths::from_rel_string(&self.root, rel_path);
        // Checked before anything else, and checked against the *filename*
        // rather than against the list above: this function removes a file, and
        // the argument comes over a bridge. A caller that got the path wrong
        // must fail here rather than delete a node.
        let name = sibling
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(conflict::parse)
            .ok_or_else(|| Error::NotAConflict(sibling.clone()))?;
        if !sibling.is_file() {
            return Err(Error::NotAConflict(sibling));
        }

        let target = sibling.with_file_name(name.target_file_name());
        let target_rel = target
            .strip_prefix(&self.root)
            .map(paths::to_rel_string)
            .unwrap_or_else(|_| paths::to_rel_string(&target));

        // `None` covers the node file having been deleted since the card was
        // drawn, which is a change like any other and gets the same answer.
        let current = atomic::read_stamped(&target)?;
        let current_hash = current.as_ref().map(|(_, s)| s.hash.as_str()).unwrap_or("");
        if current_hash != expected_hash {
            return Ok(Resolved::Stale);
        }

        match keep {
            // Nothing is written: the winner is already the file on disk. The
            // guard above is what makes this safe — it is the reason a bare
            // delete here cannot discard a version the user did not reject.
            Keep::Current => {
                // Read before the delete, obviously, and kept even if the
                // recording below finds nobody to attribute it to: the hash of
                // the bytes being refused is the whole content of the decision.
                let refused = atomic::read_stamped(&sibling)?.map(|(_, stamp)| stamp.hash);
                self.remove_sibling(&sibling)?;
                // After the removal, not before. A refusal recorded for a card
                // that is still on screen would be a row describing a decision
                // that did not land — and the order that matters is the same one
                // `Keep::Parked` uses below: the index only ever learns about a
                // change the folder already made.
                if let Some(refused) = refused {
                    self.record_refusal(&name, &target_rel, &refused)?;
                }
                Ok(Resolved::Done)
            }
            Keep::Parked => {
                let Some((parked, _)) = atomic::read_stamped(&sibling)? else {
                    return Err(Error::NotAConflict(sibling));
                };
                let expected = current.map(|(_, stamp)| stamp);

                // Back through `guarded_write` rather than a plain write. The
                // hash check above closed the window the user spent reading;
                // this closes the microseconds between that check and the
                // rename, which on a share is a real window and the one place a
                // resolution could still clobber.
                match atomic::guarded_write(
                    &self.root,
                    &target,
                    &parked,
                    expected.as_ref(),
                    &self.peer,
                )? {
                    WriteOutcome::Written(stamp) => {
                        match markdown::from_markdown(&parked, &target) {
                            Ok(node) => {
                                self.index.upsert_node(&node, &target_rel, &stamp)?;
                                self.index.clear_corrupt(&target_rel)?;
                            }
                            // The user chose a version that does not parse —
                            // a sibling a sync client truncated, most likely.
                            // It is still their choice, so it lands; the
                            // navigator says so rather than the app refusing.
                            Err(e) => {
                                let why = self.relative_message(&e.to_string());
                                self.index.mark_corrupt(&target_rel, &why)?;
                            }
                        }
                        // Only after the winner is safely on disk. The other
                        // order loses everything if the write then fails.
                        self.remove_sibling(&sibling)?;
                        Ok(Resolved::Done)
                    }
                    WriteOutcome::Conflict { conflict_path, .. } => {
                        let rel = conflict_path
                            .strip_prefix(&self.root)
                            .map(paths::to_rel_string)
                            .unwrap_or_else(|_| paths::to_rel_string(&conflict_path));
                        Ok(Resolved::Conflict { conflict_path: rel })
                    }
                }
            }
        }
    }

    /// Write down that a person refused these exact bytes, so the next sync does
    /// not park them again.
    ///
    /// A conflict does not move the base — deliberately, and #80's argument for
    /// that stands — so the same disagreement is rediscovered every round and the
    /// card the user has just dismissed comes straight back (#89). This is the
    /// other half: `sync_rejected` holds the refusal, and
    /// [`crate::apply::already_refused`] reads it.
    ///
    /// ## Finding the peer, when all we have is an alias
    ///
    /// The refusal is keyed on a full 64-hex `EndpointId`, because an alias is
    /// twenty-eight bits of a public key and this codebase's standing rule is
    /// that an alias is displayed and never decides anything. But a conflict
    /// sibling only *carries* an alias — that is the whole of what its filename
    /// can carry, since the name is a contract with other machines that must read
    /// the same on all of them — and there is no table mapping one back to an id,
    /// by design (#76).
    ///
    /// So the alias is inverted the only honest way: re-derive it, with the same
    /// pure function, over every peer id this project already knows, and take the
    /// answer only if **exactly one** matches. The decision is then keyed on the
    /// full id, so the alias has narrowed a candidate set rather than authorised
    /// anything.
    ///
    /// Both failure modes are deliberately the same failure, and it is the
    /// harmless one — nothing is recorded, and the card comes back exactly as it
    /// does today:
    ///
    /// - **No candidate matches.** The sibling came from a peer this project has
    ///   never settled or refused anything with, or from a build with a different
    ///   derivation, or the name was hand-made. Guessing here would attach a
    ///   person's decision to a machine that had nothing to do with it.
    /// - **Several candidates match.** Two ids sharing an alias is a thirty-two
    ///   bit coincidence, or somebody grinding keypairs. Recording for all of
    ///   them would let a ground key suppress one specific version of one node
    ///   from an honest peer, which is a lost edit — small, targeted, and silent.
    ///   Recording for none costs one redundant card.
    ///
    /// The node also has to be one the index knows, because a refusal is keyed by
    /// node id and a sibling beside an unindexed file has no id to key on. That
    /// is the same shrug for the same reason.
    ///
    /// A caller that *does* hold a real peer id — `wobu-sync`, or a shell command
    /// wired to a card that carries one — should use [`reject_from_peer`] and
    /// skip all of the above.
    ///
    /// [`reject_from_peer`]: Project::reject_from_peer
    pub(super) fn record_refusal(
        &self,
        name: &conflict::SiblingName,
        target_rel: &str,
        refused_hash: &str,
    ) -> Result<()> {
        let Some(alias) = name.peer.as_deref() else { return Ok(()) };
        let Some((node_id, _)) = self.index.node_at_rel_path(target_rel)? else { return Ok(()) };

        let mut matches = self
            .index
            .known_peers()?
            .into_iter()
            .filter(|peer_id| apply::alias_of(peer_id) == alias);
        let (Some(peer_id), None) = (matches.next(), matches.next()) else { return Ok(()) };

        self.index.record_rejection(&peer_id, node_id, refused_hash)
    }

    /// Refuse one version of one node from a peer named by its id.
    ///
    /// The seam without the guesswork, for a caller that knows which machine sent
    /// the bytes. [`resolve_conflict`] cannot know — it is handed a filename, and
    /// a filename holds an alias — so it re-derives; this takes the id straight.
    ///
    /// `hash` is the full BLAKE3 hex of the version being refused, as
    /// [`crate::apply::Outgoing::hash`] and `Stamp` spell it. A truncated
    /// `source_version` here would suppress a later rename from the same peer,
    /// because a version is blind to names.
    ///
    /// Nothing is written to the folder and nothing is deleted. A refusal only
    /// ever turns a future conflict into "do nothing" — it can never authorise a
    /// write — so this is safe to call for a decision that turns out to be
    /// redundant, and idempotent for one made twice.
    ///
    /// [`resolve_conflict`]: Project::resolve_conflict
    pub fn reject_from_peer(&self, peer_id: &str, node_id: Id, hash: &str) -> Result<()> {
        self.index.record_rejection(peer_id, node_id, hash)
    }

    /// The single `remove_file` call that may name a conflict sibling.
    ///
    /// Trivial on purpose: keeping it as one named function means a grep for
    /// callers is a complete audit of what can delete one, which is the only
    /// way to keep that promise true as the crate grows.
    pub(super) fn remove_sibling(&self, sibling: &Path) -> Result<()> {
        match std::fs::remove_file(sibling) {
            Ok(()) => Ok(()),
            // Already gone — a collaborator resolved the same card. The user
            // asked for it to not be there and it is not there.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::io(sibling, e)),
        }
    }
}

struct TransferPublishFailure {
    index: usize,
    deduped: usize,
    error: Error,
}

fn publish_transfer_items<T>(
    items: &[T],
    mut publish: impl FnMut(&T) -> Result<bool>,
) -> std::result::Result<usize, TransferPublishFailure> {
    let mut deduped = 0usize;
    for (index, item) in items.iter().enumerate() {
        match publish(item) {
            Ok(was_deduped) => deduped += usize::from(was_deduped),
            Err(error) => return Err(TransferPublishFailure { index, deduped, error }),
        }
    }
    Ok(deduped)
}
