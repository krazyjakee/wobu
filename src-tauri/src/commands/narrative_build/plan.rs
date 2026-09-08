use super::*;
use wobu_narrative_context::{Input as ContextInput, Options, Selection, resolve_linked};
use wobu_narrative_generation::REQUEST_VERSION;

pub fn build(
    project: &mut Project,
    input: wobu_narrative_build::Input,
    provider: &str,
    model: &str,
) -> CommandResult<Build> {
    if project.is_read_only() {
        return Err(wobu_store::Error::ReadOnly.into());
    }
    let snapshot = project.narrative_dependency_snapshot()?;
    let affected: BTreeMap<_, _> = project
        .narrative_dependencies()?
        .diff(&snapshot.index())
        .into_iter()
        .map(|a| (a.target.variant(), a))
        .collect();
    let report = super::super::narrative_preview::compile_project(project, input.commands.clone());
    let (mut graph, mut diagnostics) = match report {
        Ok(report) => (
            report.graph.map(|g| g.hash()),
            report
                .diagnostics
                .into_iter()
                .filter(|d| d.severity == wobu_narrative_compiler::Severity::Error)
                .map(|d| format!("{}: {}", d.code, d.message))
                .collect::<Vec<_>>(),
        ),
        Err(error) => (None, vec![error.message]),
    };
    let records = match records::RecordSet::load(project) {
        Ok(records) => records,
        Err(error) => {
            diagnostics
                .push(format!("Retained generation evidence needs repair: {}", error.message));
            graph = None;
            records::RecordSet::default()
        }
    };
    let mut reusable = BTreeMap::new();
    for request in records.requests.values() {
        if request.version != REQUEST_VERSION {
            continue;
        }
        let attempts = match records.attempts(request) {
            Ok(attempts) => attempts,
            Err(error) => {
                diagnostics
                    .push(format!("Retained attempt evidence needs repair: {}", error.message));
                graph = None;
                continue;
            }
        };
        for (_, receipt) in attempts {
            if let Receipt::NarrativeGenerationAttempt {
                status: AttemptStatus::Succeeded,
                raw_accepted_output: Some(raw),
                candidate: Some(candidate),
                ..
            } = receipt
            {
                if request.validate_output(&raw).is_ok_and(|validated| validated == candidate) {
                    reusable.entry(wobu_narrative_build::reuse_key(request)).or_insert(request);
                } else {
                    diagnostics.push(format!(
                        "Request {} has invalid retained output; it cannot be reused.",
                        request.request_id
                    ));
                }
            }
        }
    }
    for id in &input.containers {
        if !snapshot.scenes.iter().any(|s| s.id == *id)
            && !snapshot.texts.iter().any(|a| a.id.raw() == id.raw())
        {
            diagnostics.push(format!("Selected container {id} no longer exists."));
        }
    }
    let linked = snapshot.scenes.iter().cloned().map(|s| (s.id, s)).collect();
    let mut containers = snapshot.scenes.clone();
    containers.extend(snapshot.texts.iter().map(|a| a.editorial_scene()));
    containers.sort_by_key(|s| s.id);
    let mut result = Build {
        version: wobu_narrative_build::VERSION,
        id: wobu_core::new_id(),
        scope: input.scope,
        provider: provider.into(),
        model: model.into(),
        items: vec![],
        diagnostics: diagnostics.clone(),
    };
    let mut requests = Vec::new();
    for scene in &containers {
        if !input.containers.is_empty() && !input.containers.contains(&scene.id) {
            continue;
        }
        for beat in &scene.beats {
            for slot in &beat.dialogue {
                let variants: Vec<_> = if slot.variants.is_empty() {
                    vec![None]
                } else {
                    slot.variants.iter().map(Some).collect()
                };
                for variant in variants {
                    let missing = variant.is_none_or(|v| v.text.body.trim().is_empty());
                    let why = variant.and_then(|v| affected.get(&v.id));
                    if !match input.scope {
                        Scope::Missing => missing,
                        Scope::Affected => why.is_some(),
                        Scope::AllSelected => true,
                    } {
                        continue;
                    }
                    if result.items.len() >= wobu_narrative_build::MAX_ITEMS {
                        return Err(invalid(
                            "Build exceeds 10,000 items. Select fewer containers.",
                        ));
                    }
                    let target = Selection {
                        scene: scene.id,
                        beat: beat.id,
                        slot: slot.id,
                        variant: variant.map(|v| v.id),
                    };
                    let mut item = wobu_narrative_build::Item {
                        id: wobu_core::new_id(),
                        target: target.clone(),
                        asset: scene.supporting_text.is_some(),
                        label: format!(
                            "{} · {} · {}",
                            scene.name,
                            beat.title,
                            variant
                                .map(|v| v.text.body.as_str())
                                .unwrap_or("Empty line")
                                .chars()
                                .take(64)
                                .collect::<String>()
                        ),
                        action: wobu_narrative_build::action(
                            scene.supporting_text.as_ref().map(|a| a.policy),
                            slot.policy,
                            variant.map(|v| v.text.lifecycle.policy),
                        ),
                        reasons: why.map(|a| a.explanations()).unwrap_or_default(),
                        diagnostics: vec![],
                        request_id: None,
                        reusable: false,
                        candidate_variant_id: None,
                        state: input.state.clone(),
                    };
                    if item.action == Action::Locked {
                        item.diagnostics.push("Locked content cannot enter generation.".into());
                    } else if graph.is_none() {
                        item.action = Action::Blocked;
                        item.diagnostics = diagnostics.clone();
                    } else {
                        let context = resolve_linked(
                            ContextInput {
                                scene,
                                world: &snapshot.world,
                                schema: &snapshot.schema,
                                characters: &snapshot.characters,
                            },
                            Options {
                                selection: target.clone(),
                                state: item.state.clone(),
                                token_budget: input.token_budget,
                            },
                            &linked,
                        );
                        if !context.ready {
                            item.action = Action::Blocked;
                            item.diagnostics = context
                                .diagnostics
                                .iter()
                                .filter(|d| d.blocking)
                                .map(|d| d.message.clone())
                                .collect();
                        } else {
                            let mut request =
                                generation::freeze::request(generation::freeze::Input {
                                    batch: result.id,
                                    scene,
                                    slot,
                                    target,
                                    context,
                                    graph: graph.clone().unwrap_or_default(),
                                    provider,
                                    model,
                                    max_output_tokens: input.max_output_tokens,
                                })?;
                            if let Some(previous) =
                                reusable.get(&wobu_narrative_build::reuse_key(&request))
                            {
                                request = (*previous).clone();
                                item.reusable = true;
                            }
                            item.request_id = Some(request.request_id);
                            item.candidate_variant_id = Some(request.candidate_variant_id);
                            if !item.reusable {
                                requests.push(request);
                            }
                        }
                    }
                    if why.is_some_and(|a| a.kind == wobu_narrative_deps::AffectedKind::Untracked) {
                        item.diagnostics.push("No historical dependency evidence; current inputs will be frozen explicitly.".into());
                    }
                    result.items.push(item);
                }
            }
        }
    }
    // Deleted targets remain visible diagnostics, never silently recreated.
    for item in affected.values().filter(|a| a.kind == wobu_narrative_deps::AffectedKind::Absent) {
        result.diagnostics.push(format!(
            "Recorded target {} is absent; retained evidence is preserved.",
            item.target.line()
        ));
    }
    result.validate().map_err(invalid)?;
    snapshot.check_current(project)?;
    for request in requests {
        records::save_receipt(
            project,
            request.request_id,
            "Frozen build request",
            &Receipt::NarrativeGenerationRequest { request: Box::new(request) },
        )?;
    }
    project.save_narrative_build(&result)?;
    Ok(result)
}
