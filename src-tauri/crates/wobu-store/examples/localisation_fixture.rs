//! Reproducible test project and interchange files; not a claim of translation quality.
#[allow(dead_code)]
#[path = "harbor_voices_fixture.rs"]
mod harbor;
use std::{error::Error, path::Path};
use wobu_narrative::{
    GenerationPolicy, SceneId, Text,
    review::{EditorialAction, PolicyScope},
};
use wobu_narrative_locale::{
    LocaleId, PluralCategory,
    interchange::{decode, encode},
};
use wobu_store::{Project, project::narrative_review::ReviewRequest};
pub fn create(parent: &Path) -> Result<Project, Box<dyn Error>> {
    let mut project = harbor::create(parent)?;
    let first = project.text_catalog()?.assets[0].id;
    let mut file = project.load_text_asset(first)?;
    file.asset.entries[0].lines[0].variants[0].text = Text::written(
        "{name}, you have {count:number} harbour lights.\nThe keeper said, \"Welcome.\"",
    );
    project.save_text_asset(&mut file)?;
    for asset in project.text_catalog()?.assets {
        let scene = SceneId::from_raw(asset.id.raw());
        let targets: Vec<_> = project
            .review_scene(scene, None)?
            .lines
            .iter()
            .map(|line| line.target.clone())
            .collect();
        for target in targets {
            for action in [
                EditorialAction::Approve,
                EditorialAction::Policy {
                    scope: PolicyScope::Variant,
                    policy: GenerationPolicy::Locked,
                },
            ] {
                let view = project.review_scene(scene, None)?;
                let line = view.lines.iter().find(|line| line.target == target).unwrap();
                project.apply_review(&ReviewRequest {
                    guard: view.guard.clone(),
                    target: target.clone(),
                    context_revision: line.context_revision.clone(),
                    state_json: view.state_json.clone(),
                    action,
                })?;
            }
        }
    }
    for (tag, prefix) in [("fr", "Texte de test"), ("ar", "نص تجريبي")] {
        let locale: LocaleId = tag.parse()?;
        let exported = project.locale_export(&locale, true)?;
        std::fs::write(parent.join(format!("{tag}-source.csv")), &exported)?;
        let mut rows = decode(&exported, true)?;
        for row in &mut rows {
            let text = if row.source.text.contains("{count:number}") {
                if tag == "fr" {
                    "{name}, vous avez {count:number} feux du port.\nLe gardien a dit : « Bienvenue. »".to_string()
                } else {
                    "{name}، لديك {count:number} أضواء في الميناء.\nقال الحارس: «أهلاً».".to_string()
                }
            } else {
                format!("{prefix} : {}", row.source.text)
            };
            row.forms.insert(PluralCategory::Other, text.clone());
            if row.source.text.contains("{count:number}") {
                row.forms.insert(PluralCategory::One, text);
            }
        }
        let csv = encode(&rows, true)?;
        std::fs::write(parent.join(format!("{tag}-translated.csv")), &csv)?;
        let report = project.locale_import(&csv, true)?;
        if report.applied.len() != rows.len() {
            return Err(format!("Fixture import failed: {:?}", report.diagnostics).into());
        }
        // Arabic is already reviewed; French stays Draft for the native UI walkthrough.
        if tag == "ar" {
            for row in decode(&project.locale_export(&locale, false)?, false)? {
                project.locale_approve(&row)?;
            }
        }
    }
    let (mut policy, guard) = project.locale_policy()?;
    policy.required.insert("fr".parse()?, false);
    policy.required.insert("ar".parse()?, false);
    project.save_locale_policy(policy, &guard)?;
    Ok(project)
}
#[cfg(not(test))]
fn main() -> Result<(), Box<dyn Error>> {
    let parent = std::env::args()
        .nth(1)
        .ok_or("Pass an existing parent directory for the fresh fixture.")?;
    let project = create(Path::new(&parent))?;
    println!("{}", project.root().display());
    Ok(())
}
