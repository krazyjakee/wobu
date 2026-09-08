//! The reverse index: from a source address back to the lines that read it.
//!
//! Two maps, and they do different jobs.
//!
//! - `sets` is the record of what each line was written against. It is the only
//!   thing a comparison needs, and a comparison over it is exact.
//! - `edges` is the reverse view: address → the lines that read it. It answers
//!   *which lines could possibly care about this edit* without looking at any of
//!   them, which is what a build planner (#169) needs and what keeps a save in
//!   a thousand-line project from re-deriving the whole project.
//!
//! `edges` is derived from `sets` and is therefore never authoritative. That is
//! the same rule the SQLite index lives by — it holds no canonical data and
//! deleting it is always safe — applied one level up: an index that has been
//! lost is rebuilt by capturing every line again from the project folder, and
//! [`DependencyIndex::rebuild`] is the only constructor there is, so there is no
//! way to assemble one that its own inputs do not justify. `tests/rebuild.rs`
//! asserts the equivalence: an index built once and an index thrown away and
//! rebuilt are the same index, edges included.
//!
//! The candidate set is deliberately a *superset* of the affected set.
//! [`DependencyIndex::candidates`] answers from edges alone and can name a line
//! whose recorded hash at that address happens to be unchanged;
//! [`DependencyIndex::diff`] then settles it exactly. Erring the other way — a
//! candidate filter that could exclude an affected line — would produce a build
//! that silently skipped work, which is the failure this whole issue exists to
//! prevent.

use std::collections::{BTreeMap, BTreeSet};

use wobu_narrative::VariantId;

use crate::change::{Affected, AffectedKind, compare};
use crate::{DependencySet, TargetRef};

/// Every reverse-index key one dependency set contributes.
///
/// Prefixed by kind rather than left bare, so that a query named `state` and a
/// field addressed `state/...` cannot collide into one edge. The prefixes are
/// part of the stored index, so changing one is an index-version change.
pub fn edge_keys(set: &DependencySet) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for address in set.fields.keys() {
        keys.insert(format!("field:{address}"));
    }
    for (name, query) in &set.queries {
        keys.insert(format!("query:{name}"));
        for member in query.members.keys() {
            keys.insert(format!("member:{name}/{member}"));
        }
    }
    for (component, _) in set.versions.components() {
        keys.insert(format!("version:{component}"));
    }
    if let Some(producer) = &set.producer {
        keys.insert(format!("producer:{}/{}", producer.provider, producer.model));
    }
    keys
}

/// What every line in a project was written against, plus the reverse view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DependencyIndex {
    sets: BTreeMap<VariantId, DependencySet>,
    edges: BTreeMap<String, BTreeSet<VariantId>>,
}

impl DependencyIndex {
    /// Build an index from captured sets — the only way to make one.
    ///
    /// Keyed by [`VariantId`] because wording identities are globally unique
    /// ULIDs, so a scene line and a bark can never collide. A repeated id is a
    /// duplicate-identity error the compiler already refuses by name
    /// (`duplicate_id`); here the last set wins, because silently keeping the
    /// first would make the index disagree with the compiler about which
    /// document a broken project contains.
    pub fn rebuild(sets: impl IntoIterator<Item = DependencySet>) -> DependencyIndex {
        let mut index = DependencyIndex::default();
        for set in sets {
            let variant = set.variant();
            if let Some(previous) = index.sets.insert(variant, set) {
                for key in edge_keys(&previous) {
                    if let Some(targets) = index.edges.get_mut(&key) {
                        targets.remove(&variant);
                    }
                }
            }
            for key in edge_keys(&index.sets[&variant]) {
                index.edges.entry(key).or_default().insert(variant);
            }
        }
        index.edges.retain(|_, targets| !targets.is_empty());
        index
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    pub fn len(&self) -> usize {
        self.sets.len()
    }

    pub fn sets(&self) -> impl Iterator<Item = &DependencySet> {
        self.sets.values()
    }

    pub fn get(&self, variant: VariantId) -> Option<&DependencySet> {
        self.sets.get(&variant)
    }

    /// The reverse view, for a caller that wants to store or inspect it.
    pub fn edges(&self) -> &BTreeMap<String, BTreeSet<VariantId>> {
        &self.edges
    }

    /// The fingerprint of every line, which is what a cache is keyed on.
    pub fn fingerprints(&self) -> BTreeMap<VariantId, String> {
        self.sets.iter().map(|(id, set)| (*id, set.fingerprint())).collect()
    }

    /// Which lines could be affected by a change at these keys.
    ///
    /// A superset of the affected set, answered from edges alone; see the
    /// module documentation for why the error is deliberately in that
    /// direction.
    pub fn candidates(&self, keys: &BTreeSet<String>) -> BTreeSet<VariantId> {
        keys.iter()
            .filter_map(|key| self.edges.get(key))
            .flat_map(|targets| targets.iter().copied())
            .collect()
    }

    /// Exactly which lines differ between this index and a freshly captured
    /// one, and why.
    ///
    /// `self` is the record — what results were produced under — and `current`
    /// is the project as it is now. Nothing here writes anything: the answer is
    /// a report, and deciding what to do about it is the caller's.
    pub fn diff(&self, current: &DependencyIndex) -> Vec<Affected> {
        let variants: BTreeSet<_> = self.sets.keys().chain(current.sets.keys()).collect();
        let mut affected = Vec::new();
        for variant in variants {
            match (self.sets.get(variant), current.sets.get(variant)) {
                (Some(previous), Some(fresh)) => {
                    let reasons = compare(previous, fresh);
                    if !reasons.is_empty() {
                        affected.push(Affected {
                            target: fresh.target.clone(),
                            kind: AffectedKind::Changed,
                            before: Some(previous.fingerprint()),
                            after: Some(fresh.fingerprint()),
                            reasons,
                        });
                    }
                }
                (None, Some(fresh)) => affected.push(Affected {
                    target: fresh.target.clone(),
                    kind: AffectedKind::Untracked,
                    before: None,
                    after: Some(fresh.fingerprint()),
                    reasons: Vec::new(),
                }),
                (Some(previous), None) => affected.push(Affected {
                    target: previous.target.clone(),
                    kind: AffectedKind::Absent,
                    before: Some(previous.fingerprint()),
                    after: None,
                    reasons: Vec::new(),
                }),
                (None, None) => {}
            }
        }
        affected
    }

    /// The lines whose recorded target names this container, for a caller that
    /// has to open a document to act on an affected line.
    pub fn targets_in<'a>(
        &'a self,
        container: &'a str,
    ) -> impl Iterator<Item = &'a TargetRef> + 'a {
        self.sets.values().map(|set| &set.target).filter(move |target| match target {
            TargetRef::SceneLine { scene, .. } => scene.to_string() == container,
            TargetRef::TextLine { asset, .. } => asset.to_string() == container,
        })
    }
}
