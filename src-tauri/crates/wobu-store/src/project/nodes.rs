//! Reading and writing the Markdown files a project is made of.
//!
//! Every write goes through `write_node`, which is where the stamp check, the
//! conflict park and the index update happen together. A caller that wrote the
//! file itself would get two of those three and lose the third silently.

use std::path::Path;

use wobu_core::{
    Description, DescriptionState, EnhanceStamp, Id, Link, LinkEdge, LinkRole, Node, NodeKind,
    NodeSummary, SourceStamp, kind_def,
};

use super::*;
use crate::atomic::{self, WriteOutcome};
use crate::error::{Error, Result};
use crate::index::{CorruptFile, Touched};
use crate::markdown;
use crate::paths;

impl Project {
    pub fn list_nodes(&self) -> Result<Vec<NodeSummary>> {
        self.index.list_nodes()
    }

    /// Every explicit influence edge, read from the local derived index.
    pub fn node_links(&self) -> Result<Vec<LinkEdge>> {
        self.index.links()
    }

    /// A parser message with the project root taken out of it.
    ///
    /// `wobu-core` and the YAML parser both name the file by absolute path,
    /// which is right for a log and wrong for anything shown to a person: the
    /// leading half of it is the user's home directory, and this string ends up
    /// on screen and, sooner or later, pasted into a bug report. Everything
    /// else about the message is kept verbatim, because "expected a mapping at
    /// line 4" is the part that says what to fix.
    pub(super) fn relative_message(&self, message: &str) -> String {
        let root = self.root.to_string_lossy();
        let stripped = message.replace(&format!("{root}/"), "");
        // Windows renders the same path with backslashes.
        stripped.replace(&format!("{}\\", root.replace('/', "\\")), "")
    }

    /// Node files that are on disk and cannot be parsed.
    ///
    /// Read alongside [`list_nodes`](Self::list_nodes) rather than folded into
    /// it: a truncated file may never have had a node row, so there is nothing
    /// to fold it onto.
    pub fn corrupt_files(&self) -> Result<Vec<CorruptFile>> {
        self.index.corrupt_files()
    }

    pub fn get_node(&self, id: Id) -> Result<Node> {
        let rel = self.index.rel_path_of(id)?.ok_or_else(|| Error::NoSuchNode(id.to_string()))?;
        let path = paths::from_rel_string(&self.root, &rel);
        let Some((text, _)) = atomic::read_stamped(&path)? else {
            // The index says this node exists and the file says otherwise. If
            // the whole folder has gone, believe the index: telling the user
            // their character does not exist, when it is sitting safely on a
            // NAS that happens to be unplugged, is both wrong and alarming.
            return Err(if self.is_present() {
                Error::NoSuchNode(id.to_string())
            } else {
                Error::Disconnected
            });
        };
        markdown::from_markdown(&text, &path)
    }

    /// The exact node version a long-running local task read before it started.
    pub fn node_stamp(&self, id: Id) -> Result<Option<atomic::Stamp>> {
        self.index.stamp_of(id)
    }

    /// Every node in the project, whole, for the influence engine.
    ///
    /// `wobu-influence` is pure by design: it borrows already-loaded `Node`s and
    /// does no IO, so that `prompt_compile` — which runs on every drag of a
    /// weight slider — stays sub-millisecond. Somebody has to hold those nodes,
    /// and this is the only place that can. The shell holds the `Project` under
    /// a mutex it must not do file IO beneath (`state.rs`), and a cache anywhere
    /// else would have to be told when a project closes and when a different one
    /// opens; here it is a field of the thing that *is* the open project, so a
    /// close drops it and no other project can ever be served from it.
    ///
    /// **Nothing here reads the project folder.** The nodes are rehydrated from
    /// the index's `doc` column, which lives in local app data, so the answer
    /// costs the same whether the world is on an SSD or on an SMB share that is
    /// currently unplugged — and the Inspector has to keep working in the second
    /// case, which is the whole promise of `docs/07-file-shares.md`. The index
    /// being one reconcile behind the folder is the same staleness the navigator
    /// already renders with, and the same event clears it.
    ///
    /// Built in full once, then patched: every writer of a node row records the
    /// id it touched, so a save, or a collaborator's edit arriving through
    /// `reconcile`, re-reads one row rather than the world. The full build is the
    /// state after an open, a rescan or an index rebuild — see `index::Touched`.
    ///
    /// The cost is real and worth stating: this holds the whole world in memory
    /// for as long as the project is open, at roughly the size of the Markdown
    /// it came from. A world of two thousand entities with a couple of kilobytes
    /// of prose each is a few megabytes. That is the price of an Inspector that
    /// does not stutter, and it is paid only by projects that open the panel.
    pub fn world_nodes(&mut self) -> Result<&[Node]> {
        match self.index.take_touched() {
            Touched::Everything => self.world = self.index.nodes()?,
            Touched::These(ids) => {
                for id in ids {
                    // Kept sorted by id, which is the order `Index::nodes`
                    // returns and the order `World` needs — it picks the Style
                    // Guide by lowest id, and a project must not resolve
                    // differently depending on which node was saved last.
                    let at = self.world.binary_search_by_key(&id, |n| n.id);
                    match (self.index.node(id)?, at) {
                        (Some(node), Ok(at)) => self.world[at] = node,
                        (Some(node), Err(at)) => self.world.insert(at, node),
                        (None, Ok(at)) => drop(self.world.remove(at)),
                        // Touched and gone and never held: a node created and
                        // deleted between two reads of this.
                        (None, Err(_)) => {}
                    }
                }
            }
        }
        Ok(&self.world)
    }

    /// Clone one complete, reconciled view for the static wiki renderer.
    ///
    /// The caller performs `reconcile` first so it can emit `world:changed` if
    /// the export noticed an external edit. Rendering and image copies happen
    /// after this snapshot releases the shell's project lock.
    pub fn wiki_snapshot(&mut self) -> Result<crate::wiki::WikiSnapshot> {
        let corrupt = self.corrupt_files()?.len();
        let conflicts = self.conflicts()?.len();
        if corrupt > 0 || conflicts > 0 {
            return Err(Error::ExportBlocked { corrupt, conflicts });
        }
        let nodes = self.world_nodes()?.to_vec();
        let assets = self.list_assets()?;
        Ok(crate::wiki::WikiSnapshot::new(self.root.clone(), self.meta.name.clone(), nodes, assets))
    }

    // ── writing ──────────────────────────────────────────────────────────

    pub fn create_node(
        &mut self,
        kind: NodeKind,
        name: &str,
        parent_id: Option<Id>,
    ) -> Result<Node> {
        self.ensure_writable()?;

        let def = kind_def(kind);
        if def.singleton && self.index.singleton_of(kind)?.is_some() {
            return Err(Error::Core(wobu_core::Error::DuplicateSingleton { kind: kind.as_str() }));
        }

        let mut node = Node::new(kind, name)?;
        node.parent_id = parent_id;

        // Two nodes of a kind may share a display name, but not a filename.
        let taken = self.index.slugs_in_kind(kind)?;
        node.slug = wobu_core::unique_slug(&node.slug, &|s| taken.iter().any(|t| t == s));

        node.validate()?;
        self.validate_parent(&node, parent_id)?;

        match self.write_node(&node, None)? {
            SaveOutcome::Saved(saved) => Ok(*saved),
            SaveOutcome::Conflict { conflict_path } => {
                Err(Error::AlreadyExists(paths::from_rel_string(&self.root, &conflict_path)))
            }
        }
    }

    /// Save an edited node, refusing to clobber a concurrent change.
    pub fn save_node(&mut self, mut node: Node) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        node.validate()?;
        self.validate_parent(&node, node.parent_id)?;
        node.touch();

        let expected = self.index.stamp_of(node.id)?;
        self.write_node(&node, expected.as_ref())
    }

    /// Publish a long-running task's result only if the node is still the
    /// version the task began from. The incoming version is parked as an
    /// ordinary conflict when somebody edited during the run.
    pub fn save_node_expected(
        &mut self,
        mut node: Node,
        expected: &atomic::Stamp,
    ) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        node.validate()?;
        self.validate_parent(&node, node.parent_id)?;
        node.touch();
        self.write_node(&node, Some(expected))
    }

    /// Persist the entity identity seed through the same guarded node write as
    /// every other shared edit. `None` explicitly clears the lock.
    pub fn set_locked_seed(&mut self, id: Id, seed: Option<u64>) -> Result<SaveOutcome> {
        let mut node = self.get_node(id)?;
        node.locked_seed = seed;
        self.save_node(node)
    }

    /// Land the result of an enhance, stamping what it was enhanced from.
    ///
    /// The stamp is the point. Without it there is nothing in the file that
    /// says what the model was shown, and "is this description still current"
    /// has no answer at all — so this is the only supported way to write a
    /// machine description, and it is a method on `Project` rather than
    /// something the caller assembles so that a stamp cannot be forgotten.
    ///
    /// `sources` is `wobu_influence::resolve`'s answer for this node, passed
    /// straight through. Taking it rather than recomputing it is what keeps
    /// staleness and prompt compilation talking about the same graph: there is
    /// one definition of "upstream" in Wobu and it is the walk that builds the
    /// prompt. The subject is dropped from it here rather than at the call
    /// site, because `resolve` includes the subject in its own stack and
    /// stamping a node against its own description would make it stale the
    /// instant it was written.
    ///
    /// **A hand-edited description is never overwritten silently.** `force` is
    /// the user answering the question the UI raised, not a default.
    pub fn accept_enhanced(
        &mut self,
        id: Id,
        description: Description,
        sources: &[Id],
        force: bool,
    ) -> Result<Enhanced> {
        self.ensure_writable()?;

        let mut node = self.get_node(id)?;
        if node.description_is_hand_written() && !force {
            return Ok(Enhanced::RefusedEdit(Box::new(node)));
        }

        let description = description.normalised_for(node.kind);
        let empty = description.is_empty();
        node.description = (!empty).then_some(description);
        // An enhance that produced nothing is not a fresh description, and
        // recording one would hide the failure behind a state that says the
        // node has been described.
        node.description_state =
            if empty { DescriptionState::None } else { DescriptionState::Fresh };

        let mut stamp = EnhanceStamp::default();
        for source in sources.iter().filter(|s| **s != id) {
            // A source the index cannot produce is one deleted between the
            // resolve and this call. Stamping a version we never read would
            // claim the description saw something it did not.
            if let Some(source_node) = self.index.node(*source)? {
                stamp.sources.push(SourceStamp {
                    node: *source,
                    version: crate::index::source_version(&source_node),
                });
            }
        }
        stamp.subject = crate::index::subject_version(&node);
        node.enhanced_from = Some(stamp);

        Ok(match self.save_node(node)? {
            SaveOutcome::Saved(node) => Enhanced::Saved(node),
            SaveOutcome::Conflict { conflict_path } => Enhanced::Conflict { conflict_path },
        })
    }

    pub fn move_node(&mut self, id: Id, new_parent_id: Option<Id>) -> Result<()> {
        let mut node = self.get_node(id)?;
        if node.parent_id == new_parent_id {
            return Ok(());
        }
        node.parent_id = new_parent_id;
        // save_node re-validates, which is where the cycle check happens.
        match self.save_node(node)? {
            SaveOutcome::Saved(_) => Ok(()),
            SaveOutcome::Conflict { conflict_path } => {
                Err(Error::AlreadyExists(paths::from_rel_string(&self.root, &conflict_path)))
            }
        }
    }

    // ── node links ───────────────────────────────────────────────────────

    /// Add an explicit influence edge, or replace the same `(target, role)`.
    ///
    /// The registry is checked here as well as in the picker. A webview can be
    /// stale after an app update, and an edge the current kind does not declare
    /// would otherwise be writable but impossible to add again after removal.
    pub fn add_node_link(
        &mut self,
        node_id: Id,
        to_id: Id,
        role: LinkRole,
        weight: Option<f32>,
        enabled: Option<bool>,
    ) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        if self.index.node(to_id)?.is_none() {
            return Err(Error::NoSuchNode(to_id.to_string()));
        }

        let mut node = self.get_node(node_id)?;
        self.require_link_role(&node, role)?;
        let mut link = Link::new(to_id, role);
        if let Some(weight) = weight {
            link.weight = weight;
        }
        if let Some(enabled) = enabled {
            link.enabled = enabled;
        }
        let link = link.clamped();

        match node.links.iter_mut().find(|item| item.to_id == to_id && item.role == role) {
            Some(existing) => *existing = link,
            None => node.links.push(link),
        }
        self.save_node(node)
    }

    /// Remove exactly one explicit edge. `parent_id` is deliberately not
    /// reachable here: it is an implicit relationship edited by `move_node`.
    pub fn remove_node_link(
        &mut self,
        node_id: Id,
        to_id: Id,
        role: LinkRole,
    ) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        let mut node = self.get_node(node_id)?;
        let before = node.links.len();
        node.links.retain(|item| item.to_id != to_id || item.role != role);
        if node.links.len() == before {
            return Err(Error::NoSuchNodeLink {
                target: to_id.to_string(),
                role: role.as_str().to_string(),
            });
        }
        self.save_node(node)
    }

    /// Re-weight or mute one explicit edge without replacing its other state.
    pub fn update_node_link(
        &mut self,
        node_id: Id,
        to_id: Id,
        role: LinkRole,
        weight: Option<f32>,
        enabled: Option<bool>,
    ) -> Result<SaveOutcome> {
        self.ensure_writable()?;
        let mut node = self.get_node(node_id)?;
        let Some(link) =
            node.links.iter_mut().find(|item| item.to_id == to_id && item.role == role)
        else {
            return Err(Error::NoSuchNodeLink {
                target: to_id.to_string(),
                role: role.as_str().to_string(),
            });
        };
        if let Some(weight) = weight {
            link.weight = weight.clamp(0.0, 1.0);
        }
        if let Some(enabled) = enabled {
            link.enabled = enabled;
        }
        self.save_node(node)
    }

    /// Everything explicitly pointing at this node, from the local index.
    pub fn node_backlinks(&self, id: Id) -> Result<Vec<LinkEdge>> {
        if self.index.node(id)?.is_none() {
            return Err(Error::NoSuchNode(id.to_string()));
        }
        self.index.backlinks(id)
    }

    pub(super) fn require_link_role(&self, node: &Node, role: LinkRole) -> Result<()> {
        if !kind_def(node.kind).default_link_roles.contains(&role) {
            return Err(Error::InvalidNodeLinkRole {
                kind: node.kind.as_str().to_string(),
                role: role.as_str().to_string(),
            });
        }
        Ok(())
    }

    /// Delete a node, promoting any children to its parent and stripping the
    /// influence edges that pointed at it.
    ///
    /// Deleting a Region should not silently take its Cities with it, and
    /// refusing outright would make the user delete a subtree leaf by leaf.
    ///
    /// The inbound links matter just as much: ULIDs are never reused, so a link
    /// left pointing at a deleted node is dead weight in someone's frontmatter
    /// forever, and the influence engine would resolve it into an empty layer
    /// card rather than nothing at all.
    pub fn delete_node(&mut self, id: Id) -> Result<()> {
        self.ensure_writable()?;

        let node = self.get_node(id)?;
        if kind_def(node.kind).singleton {
            return Err(Error::Core(wobu_core::Error::DuplicateSingleton {
                kind: node.kind.as_str(),
            }));
        }

        for child_id in self.index.children_of(id)? {
            let mut child = self.get_node(child_id)?;
            child.parent_id = node.parent_id;
            self.save_node(child)?;
        }

        // Collected first: each save_node below rewrites the index, and holding
        // a borrow across that would be reading a table we are mutating.
        let referrers: Vec<Id> =
            self.index.backlinks(id)?.into_iter().map(|edge| edge.from_id).collect();
        for from_id in referrers {
            // A referrer that is itself already gone is not an error — deleting
            // two linked nodes in either order must work.
            let Ok(mut referrer) = self.get_node(from_id) else { continue };
            referrer.links.retain(|link| link.to_id != id);
            self.save_node(referrer)?;
        }

        if let Some(rel) = self.index.rel_path_of(id)? {
            let path = paths::from_rel_string(&self.root, &rel);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io(&path, e)),
            }
        }
        self.index.remove_node(id)?;
        Ok(())
    }

    pub(super) fn write_node(
        &mut self,
        node: &Node,
        expected: Option<&atomic::Stamp>,
    ) -> Result<SaveOutcome> {
        let rel = self.rel_path(node);
        let path = paths::from_rel_string(&self.root, &rel);
        let text = markdown::to_markdown(node)?;

        // Two people saving the same words is not a conflict — nobody's text is
        // at risk — but every save re-stamps `updated_at`, so their bytes never
        // match. `guarded_write` can only compare bytes, so left to itself it
        // parks a `.conflict-` sibling whose sole difference from the winner is
        // a timestamp. That is worse than useless: it teaches people that
        // conflict files are noise, right up until one of them matters.
        if let Some(expected) = expected
            && let Some((theirs, stamp)) = self.same_words_on_disk(node, &path, expected)?
        {
            self.index.upsert_node(&theirs, &rel, &stamp)?;
            return Ok(SaveOutcome::Saved(Box::new(theirs)));
        }

        match atomic::guarded_write(&self.root, &path, &text, expected, &self.peer)? {
            WriteOutcome::Written(stamp) => {
                self.index.upsert_node(node, &rel, &stamp)?;
                Ok(SaveOutcome::Saved(Box::new(node.clone())))
            }
            WriteOutcome::Conflict { conflict_path, .. } => {
                // Pull the winner's version into the index so the UI shows what
                // is actually on disk while the conflict card is open.
                if let Ok(Some((text, stamp))) = atomic::read_stamped(&path)
                    && let Ok(theirs) = markdown::from_markdown(&text, &path)
                {
                    self.index.upsert_node(&theirs, &rel, &stamp)?;
                }
                let rel_conflict = conflict_path
                    .strip_prefix(&self.root)
                    .map(paths::to_rel_string)
                    .unwrap_or_else(|_| paths::to_rel_string(&conflict_path));
                Ok(SaveOutcome::Conflict { conflict_path: rel_conflict })
            }
        }
    }

    /// The file changed under us, but it says exactly what we were about to say.
    ///
    /// Returns the on-disk node and its stamp when it matches ours in every
    /// field but `updated_at`. The caller adopts it: the user's words are on
    /// disk, so the save has effectively happened, and there is nothing for a
    /// conflict card to offer a choice between.
    ///
    /// Comparison is by re-serialising our node with *their* timestamp and
    /// requiring the bytes to match theirs exactly. That is deliberately strict
    /// — a file hand-edited into different formatting falls through to the
    /// normal conflict path, which is the safe direction to be wrong in.
    pub(super) fn same_words_on_disk(
        &self,
        node: &Node,
        path: &Path,
        expected: &atomic::Stamp,
    ) -> Result<Option<(Node, atomic::Stamp)>> {
        // The cheap filter first, so the common case — nothing changed — costs
        // one `stat` rather than a read and a parse.
        match atomic::peek(path)? {
            Some((mtime, size)) if mtime == expected.mtime_ms && size == expected.size => {
                return Ok(None);
            }
            // Deleted under us. `guarded_write` recreates, which is right.
            None => return Ok(None),
            _ => {}
        }

        let Some((text, stamp)) = atomic::read_stamped(path)? else { return Ok(None) };
        if stamp.hash == expected.hash {
            return Ok(None);
        }
        let Ok(theirs) = markdown::from_markdown(&text, path) else { return Ok(None) };

        let mut ours = node.clone();
        ours.updated_at = theirs.updated_at;
        match markdown::to_markdown(&ours) {
            Ok(rendered) if rendered == text => Ok(Some((theirs, stamp))),
            _ => Ok(None),
        }
    }

    pub(super) fn rel_path(&self, node: &Node) -> String {
        format!("{NODES_DIR}/{}/{}.md", node.kind.dir(), node.slug)
    }

    pub(super) fn validate_parent(&self, node: &Node, parent_id: Option<Id>) -> Result<()> {
        let lookup = |id: Id| self.index.kind_and_parent(id).ok().flatten();
        wobu_core::validate_parent(node, parent_id, &lookup)?;
        Ok(())
    }
}
