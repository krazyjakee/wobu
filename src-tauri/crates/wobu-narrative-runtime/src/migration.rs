//! Explicit content migration. Normal restore never invokes this path.
use super::*;

/// The host may change state and cursor identities deliberately. Engine side
/// effects, pending command arguments and acknowledgement history cannot be
/// rewritten by a migration, because doing so would invalidate idempotency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Migration {
    pub from_graph_hash: String,
    pub to_graph_hash: String,
    pub state: State,
    pub scene: String,
    pub beat: String,
    pub selected_variant: Option<String>,
    pub visits: BTreeMap<String, u64>,
    /// Supporting text repeat state (#167). Editable for the same reason visits
    /// are: an asset removed from the new graph has to be droppable, or every
    /// save of the old story becomes unrestorable. It carries no external side
    /// effect, so unlike command acknowledgements there is nothing to
    /// invalidate by rewriting it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub texts: BTreeMap<String, crate::TextProgress>,
}

impl Runtime {
    /// Validate the original save against its original graph, explicitly call
    /// the supplied hook, then validate the entire migrated save against the
    /// destination graph. Neither input graph nor the original save is mutated.
    pub fn restore_with_migration(
        old_graph: Graph,
        new_graph: Graph,
        snapshot: Snapshot,
        migrate: impl FnOnce(Migration) -> Result<Migration>,
    ) -> Result<Self> {
        let old = Self::restore(old_graph, snapshot)?;
        let from = old.saved.graph_hash.clone();
        let to = new_graph.hash();
        let plan = migrate(Migration {
            from_graph_hash: from.clone(),
            to_graph_hash: to.clone(),
            state: old.saved.state.clone(),
            scene: old.saved.scene.clone(),
            beat: old.saved.beat.clone(),
            selected_variant: match &old.saved.phase {
                Phase::Dialogue { variant, .. } => Some(variant.clone()),
                _ => None,
            },
            visits: old.saved.visits.clone(),
            texts: old.saved.texts.clone(),
        })?;
        if plan.from_graph_hash != from || plan.to_graph_hash != to {
            return Err(Error::Incompatible);
        }
        let mut saved = old.saved;
        saved.command_graph_hash = Some(saved.command_graph_hash.unwrap_or(from));
        saved.graph_version = new_graph.version;
        saved.graph_hash = to;
        saved.state = plan.state;
        saved.scene = plan.scene;
        saved.beat = plan.beat;
        saved.visits = plan.visits;
        saved.texts = plan.texts;
        match &mut saved.phase {
            Phase::Dialogue { index, variant } => {
                let selected = plan.selected_variant.ok_or(Error::InvalidAction)?;
                let beat = new_graph
                    .scenes
                    .get(&saved.scene)
                    .and_then(|scene| scene.beats.get(&saved.beat))
                    .ok_or(Error::InvalidAction)?;
                *index = beat
                    .dialogue
                    .iter()
                    .position(|slot| slot.variants.iter().any(|v| v.id == selected))
                    .ok_or(Error::InvalidAction)?;
                *variant = selected;
            }
            _ if plan.selected_variant.is_some() => return Err(Error::InvalidAction),
            _ => {}
        }
        Self::restore(new_graph, saved)
    }
}
