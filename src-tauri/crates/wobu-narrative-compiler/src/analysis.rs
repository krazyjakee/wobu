//! Opt-in coverage gate for explicitly configured finite state models.
use super::*;
use wobu_narrative_variants::{Classification, Input, Policy};
/// Existing callers with no configured policy retain `compile` semantics.
/// Hosts must bind these policies and World to their source capture, and recheck
/// both policy records and source observations before publishing the result.
pub fn compile_with_analysis(
    scenes: &[Scene],
    texts: &[TextAsset],
    schema: &StateSchema,
    options: &CompileOptions,
    world: &wobu_narrative::WorldDocument,
    policies: &[Policy],
) -> CompileReport {
    let mut compiled = compile(scenes, texts, schema, options);
    let mut targets = BTreeSet::new();
    for policy in policies {
        let mut emit = |site, severity, code: &str, message: String| {
            compiled.diagnostics.push(CompileDiagnostic {
                scene: policy.target.scene.to_string(),
                asset: None,
                site,
                severity,
                code: code.into(),
                message,
            })
        };
        if !targets.insert(policy.target.clone()) {
            emit(
                Site::Scene,
                Severity::Error,
                "analysis_policy",
                "Duplicate analysis policy for a beat.".into(),
            );
            continue;
        }
        let report = match wobu_narrative_variants::analyze(Input { scenes, schema, world }, policy)
        {
            Ok(report) => report,
            Err(error) => {
                emit(Site::Scene, Severity::Error, "analysis_policy", error.to_string());
                continue;
            }
        };
        let displayed =
            report.rows.iter().filter(|r| r.classification == Classification::Included).count();
        if report.included.value != displayed.to_string() {
            emit(Site::Scene,Severity::Error,"analysis_coverage_limit","Known reachable configurations exceed the variant limit; raise it before compiling coverage.".into());
        }
        for reason in &report.reasons {
            emit(Site::Scene, Severity::Warning, "analysis_unknown", reason.clone());
        }
        let mut gaps = BTreeSet::new();
        let mut overlaps = BTreeSet::new();
        for row in &report.rows {
            if row.classification != Classification::Included {
                continue;
            }
            for coverage in &row.coverage {
                let site = Site::DialogueSlot { beat: policy.target.beat, slot: coverage.slot };
                if coverage.selected.is_none() && gaps.insert(coverage.slot) {
                    emit(
                        site,
                        Severity::Error,
                        "uncovered_configuration",
                        format!(
                            "No variant covers model witness {} (state {}); author an explicit fallback or conditional wording.",
                            row.id,
                            serde_json::to_string(&row.values).unwrap()
                        ),
                    );
                } else if coverage.matching.len() > 1 && overlaps.insert(coverage.slot) {
                    emit(
                        site,
                        Severity::Warning,
                        "overlapping_variants",
                        format!(
                            "Model witness {} matches {} variants. Authored first-match priority selects {}; fallback {}.",
                            row.id,
                            coverage.matching.len(),
                            coverage.selected.unwrap(),
                            coverage
                                .fallback
                                .map(|id| id.to_string())
                                .unwrap_or_else(|| "not authored".into())
                        ),
                    );
                }
            }
        }
    }
    if compiled.diagnostics.iter().any(|d| d.severity == Severity::Error) {
        compiled.graph = None;
    }
    compiled
}
