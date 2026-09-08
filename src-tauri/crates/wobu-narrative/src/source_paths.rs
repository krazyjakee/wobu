//! Semantic paths into the authored YAML tree. Offsets belong to the editor's
//! CST; paths belong to the same typed analysis that produces the diagnostic.
use crate::expr::TypeError;
use crate::scene::DestinationSite;
use crate::{
    Condition, Diagnostic, Effect, Operand, Problem, Scene, SceneCatalog, Site, StateSchema,
};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SourcePathPart {
    Key(String),
    Index(usize),
}
impl From<&str> for SourcePathPart {
    fn from(value: &str) -> Self {
        Self::Key(value.into())
    }
}
impl From<usize> for SourcePathPart {
    fn from(value: usize) -> Self {
        Self::Index(value)
    }
}
fn child(path: &[SourcePathPart], key: impl Into<SourcePathPart>) -> Vec<SourcePathPart> {
    let mut path = path.to_vec();
    path.push(key.into());
    path
}

impl Scene {
    /// Each returned path selects the responsible value, including repeated IDs
    /// and multiple independently invalid effects on the same choice.
    pub fn source_diagnostics(
        &self,
        schema: &StateSchema,
        catalog: &SceneCatalog,
    ) -> Vec<(Diagnostic, Vec<SourcePathPart>)> {
        let diagnostics = self.diagnostics(schema, catalog);
        let mut candidates: Vec<(Site, Problem, Vec<SourcePathPart>)> = Vec::new();
        let root = vec!["scene".into()];
        if let Some(entry) = &self.entry {
            condition(&mut candidates, Site::Entry, entry, schema, child(&root, "entry"));
        }
        for (bi, beat) in self.beats.iter().enumerate() {
            let bp = child(&child(&root, "beats"), bi);
            for (si, slot) in beat.dialogue.iter().enumerate() {
                let sp = child(&child(&bp, "dialogue"), si);
                for (vi, variant) in slot.variants.iter().enumerate() {
                    if let Some(when) = &variant.when {
                        condition(
                            &mut candidates,
                            Site::Variant { beat: beat.id, slot: slot.id, variant: variant.id },
                            when,
                            schema,
                            child(&child(&child(&sp, "variants"), vi), "when"),
                        );
                    }
                }
            }
            for (ci, choice) in beat.choices.iter().enumerate() {
                let site =
                    Site::Destination(DestinationSite::Choice { beat: beat.id, choice: choice.id });
                let cp = child(&child(&bp, "choices"), ci);
                if let Some(requires) = &choice.requires {
                    condition(&mut candidates, site, requires, schema, child(&cp, "requires"));
                }
                effects(&mut candidates, site, &choice.effects, schema, child(&cp, "effects"));
            }
            for (oi, outcome) in beat.outcomes.iter().enumerate() {
                let site = Site::Destination(DestinationSite::Outcome {
                    beat: beat.id,
                    outcome: outcome.id,
                });
                let op = child(&child(&bp, "outcomes"), oi);
                if let Some(when) = &outcome.when {
                    condition(&mut candidates, site, when, schema, child(&op, "when"));
                }
                effects(&mut candidates, site, &outcome.effects, schema, child(&op, "effects"));
            }
        }
        let mut duplicates = std::collections::HashMap::new();
        diagnostics
            .into_iter()
            .map(|diagnostic| {
                let path = if let Some(at) = candidates.iter().position(|(site, problem, _)| {
                    *site == diagnostic.site && *problem == diagnostic.problem
                }) {
                    candidates.remove(at).2
                } else {
                    let skip = if let Problem::DuplicateId { id, .. } = &diagnostic.problem {
                        let seen = duplicates.entry(id.clone()).or_insert(0);
                        *seen += 1;
                        *seen
                    } else {
                        0
                    };
                    self.site_path(&diagnostic, skip)
                };
                (diagnostic, path)
            })
            .collect()
    }

    fn site_path(&self, diagnostic: &Diagnostic, skip: usize) -> Vec<SourcePathPart> {
        let root = vec!["scene".into()];
        let mut found = Vec::new();
        if diagnostic.site == Site::Scene {
            return child(&root, "beats");
        }
        if diagnostic.site == Site::Entry {
            return child(&root, "entry");
        }
        for (bi, beat) in self.beats.iter().enumerate() {
            let bp = child(&child(&root, "beats"), bi);
            match diagnostic.site {
                Site::Destination(DestinationSite::Beat(id)) if id == beat.id => {
                    found.push(bp.clone())
                }
                Site::Intent { beat: id, index } if id == beat.id => {
                    found.push(child(&child(&child(&bp, "intents"), index), "subject"))
                }
                _ => {}
            }
            for (index, choice) in beat.choices.iter().enumerate() {
                if diagnostic.site
                    == Site::Destination(DestinationSite::Choice {
                        beat: beat.id,
                        choice: choice.id,
                    })
                {
                    found.push(child(&child(&bp, "choices"), index));
                }
            }
            for (index, outcome) in beat.outcomes.iter().enumerate() {
                if diagnostic.site
                    == Site::Destination(DestinationSite::Outcome {
                        beat: beat.id,
                        outcome: outcome.id,
                    })
                {
                    found.push(child(&child(&bp, "outcomes"), index));
                }
            }
            for (si, slot) in beat.dialogue.iter().enumerate() {
                let sp = child(&child(&bp, "dialogue"), si);
                if diagnostic.site == (Site::DialogueSlot { beat: beat.id, slot: slot.id }) {
                    found.push(sp.clone());
                }
                for (vi, variant) in slot.variants.iter().enumerate() {
                    if diagnostic.site
                        == (Site::Variant { beat: beat.id, slot: slot.id, variant: variant.id })
                    {
                        found.push(child(&child(&sp, "variants"), vi));
                    }
                }
            }
        }
        let path = found.get(skip).or_else(|| found.last()).cloned().unwrap_or(root);
        match diagnostic.problem {
            Problem::DuplicateId { .. } => child(&path, "id"),
            Problem::DanglingBeat { .. } | Problem::DeletedBeat { .. } => {
                child(&child(&path, "to"), "beat")
            }
            Problem::UnknownScene { .. } => child(&child(&path, "to"), "scene"),
            Problem::UnresolvedDestination => child(&path, "to"),
            Problem::MissingText => child(&path, "variants"),
            Problem::RevisionMismatch { .. } => child(&child(&path, "text"), "revision"),
            Problem::NotAParticipant { .. }
                if matches!(diagnostic.site, Site::DialogueSlot { .. }) =>
            {
                child(&child(&path, "speaker"), "entity")
            }
            _ => path,
        }
    }
}

fn condition(
    out: &mut Vec<(Site, Problem, Vec<SourcePathPart>)>,
    site: Site,
    value: &Condition,
    schema: &StateSchema,
    path: Vec<SourcePathPart>,
) {
    if let Err(error) = schema.check_condition(value) {
        let path = condition_path(value, schema, path, &error);
        out.push((site, Problem::Type(error), path));
    }
}
fn condition_path(
    value: &Condition,
    schema: &StateSchema,
    path: Vec<SourcePathPart>,
    error: &TypeError,
) -> Vec<SourcePathPart> {
    match value {
        Condition::Not(inner) => condition_path(inner, schema, child(&path, "not"), error),
        Condition::All(items) | Condition::Any(items) => {
            let key = if matches!(value, Condition::All(_)) { "all" } else { "any" };
            let index =
                items.iter().position(|item| schema.check_condition(item).is_err()).unwrap_or(0);
            items.get(index).map_or(path.clone(), |item| {
                condition_path(item, schema, child(&child(&path, key), index), error)
            })
        }
        Condition::Compare(cmp) => {
            let key = match error {
                TypeError::UndeclaredVariable(name) if name == &cmp.var => "var",
                TypeError::NotOrdered { .. } => "op",
                _ => "value",
            };
            let field = child(&child(&path, "compare"), key);
            if key == "value" { operand_path(&cmp.value, field) } else { field }
        }
        _ => path,
    }
}
fn effects(
    out: &mut Vec<(Site, Problem, Vec<SourcePathPart>)>,
    site: Site,
    values: &[Effect],
    schema: &StateSchema,
    path: Vec<SourcePathPart>,
) {
    for (index, effect) in values.iter().enumerate() {
        if let Err(error) = schema.check_effect(effect) {
            let path = child(&path, index);
            let path = match effect {
                Effect::Set(set) => {
                    let path = child(&path, "set");
                    if matches!(&error, TypeError::HostOwned(_))
                        || matches!(&error, TypeError::UndeclaredVariable(name) if name == &set.var)
                    {
                        child(&path, "var")
                    } else {
                        operand_path(&set.value, child(&path, "value"))
                    }
                }
                Effect::Add(_) => child(&child(&path, "add"), "var"),
                Effect::Command(command) => {
                    let args = child(&child(&path, "command"), "args");
                    command.args.iter().position(|arg| {
                        matches!((arg, &error), (Operand::Var(name), TypeError::UndeclaredVariable(missing)) if name == missing)
                    }).map_or(args.clone(), |index| child(&child(&args, index), "var"))
                }
            };
            out.push((site, Problem::Type(error), path));
        }
    }
}

fn operand_path(value: &Operand, path: Vec<SourcePathPart>) -> Vec<SourcePathPart> {
    child(
        &path,
        match value {
            Operand::Var(_) => "var",
            Operand::Literal(_) => "literal",
        },
    )
}

impl Scene {
    /// Organization references are authoring diagnostics, never runtime edges.
    pub fn classification_diagnostics(
        &self,
        world: &crate::WorldDocument,
    ) -> Vec<(Diagnostic, Vec<SourcePathPart>)> {
        let mut out = Vec::new();
        for (field, id, records) in
            [("act_id", self.act_id, &world.acts), ("arc_id", self.arc_id, &world.arcs)]
        {
            if let Some(id) = id
                && !records.iter().any(|record| record.id == id)
            {
                out.push((
                    Diagnostic {
                        site: Site::Scene,
                        problem: Problem::UnknownClassification { field, id },
                    },
                    vec!["scene".into(), field.into()],
                ));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for (index, id) in self.tag_ids.iter().copied().enumerate() {
            let problem = if !seen.insert(id) {
                Some(Problem::DuplicateId { noun: "tag reference", id: id.to_string() })
            } else if !world.tags.iter().any(|record| record.id == id) {
                Some(Problem::UnknownClassification { field: "tag_ids", id })
            } else {
                None
            };
            if let Some(problem) = problem {
                out.push((
                    Diagnostic { site: Site::Scene, problem },
                    vec!["scene".into(), "tag_ids".into(), index.into()],
                ));
            }
        }
        out
    }
}
