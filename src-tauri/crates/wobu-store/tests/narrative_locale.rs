use std::collections::BTreeMap;
use wobu_narrative::{
    DialogueSlot, GenerationPolicy, Name, SceneId, Speaker, Text, TextEntry, TextKind, Variant,
    review::{EditorialAction, PolicyScope},
};
use wobu_narrative_locale::{
    LocaleId, PluralCategory,
    interchange::{decode, encode},
};
use wobu_store::project::narrative_review::ReviewRequest;
use wobu_store::{Project, SourceSave};
fn fixture() -> (tempfile::TempDir, Project, wobu_narrative::TextAssetId) {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::create(dir.path(), "Locale world").unwrap();
    let mut file =
        project.create_text_asset(TextKind::Codex, "Harbor", Name::new("read").unwrap()).unwrap();
    let mut entry = TextEntry::new("Entry");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written("Hello, \"{name}\"!\nAt the harbor.")));
    entry.lines.push(slot);
    file.asset.entries.push(entry);
    project.save_text_asset(&mut file).unwrap();
    let id = file.asset.id;
    action(&mut project, id, EditorialAction::Approve);
    action(
        &mut project,
        id,
        EditorialAction::Policy { scope: PolicyScope::Slot, policy: GenerationPolicy::Locked },
    );
    (dir, project, id)
}
fn action(p: &mut Project, id: wobu_narrative::TextAssetId, action: EditorialAction) {
    let view = p.review_scene(SceneId::from_raw(id.raw()), None).unwrap();
    let line = &view.lines[0];
    let request = ReviewRequest {
        guard: view.guard.clone(),
        target: line.target.clone(),
        context_revision: line.context_revision.clone(),
        state_json: view.state_json.clone(),
        action,
    };
    p.apply_review(&request).unwrap();
}
#[test]
fn real_file_roundtrip_independent_review_history_unlock_and_stale_reimport() {
    let (_dir, mut p, id) = fixture();
    let locale: LocaleId = "ar".parse().unwrap();
    let mut rows = decode(&p.locale_export(&locale, true).unwrap(), true).unwrap();
    assert_eq!(rows.len(), 1);
    rows[0].forms.insert(PluralCategory::Other, "مرحباً، \"{name}\"!\nفي الميناء.".into());
    let csv = encode(&rows, true).unwrap();
    let path = p.root().parent().unwrap().join("arabic.csv");
    std::fs::write(&path, &csv).unwrap();
    let input = std::fs::read_to_string(path).unwrap();
    assert!(p.locale_preview(&input, true).unwrap().is_empty());
    assert_eq!(p.locale_import(&input, true).unwrap().applied.len(), 1);
    let mut reviewed = decode(&p.locale_export(&locale, false).unwrap(), false).unwrap();
    assert!(matches!(p.locale_approve(&reviewed[0]).unwrap(), SourceSave::Saved(_)));
    let translations = p.locale_translations().unwrap();
    let old = translations.values().next().unwrap();
    assert_eq!(old.history.len(), 2);
    assert!(old.latest().approved);
    assert!(p.locale_import(&input, true).unwrap().applied.is_empty());
    action(
        &mut p,
        id,
        EditorialAction::Policy { scope: PolicyScope::Slot, policy: GenerationPolicy::Edited },
    );
    action(
        &mut p,
        id,
        EditorialAction::Policy { scope: PolicyScope::Slot, policy: GenerationPolicy::Locked },
    );
    let source = p.locale_sources().unwrap().into_values().next().unwrap();
    assert!(!old.current(&source));
    assert_eq!(old.history[0].forms[&PluralCategory::Other], rows[0].forms[&PluralCategory::Other]);
    reviewed[0].translation_guard = Some(old.token());
    assert!(p.locale_approve(&reviewed[0]).is_err());
    let root = p.root().to_path_buf();
    drop(p);
    let reopened = Project::open(&root).unwrap();
    assert_eq!(reopened.locale_translations().unwrap().values().next().unwrap(), old);
}
#[test]
fn partial_import_skips_invalid_rows_and_canonical_receipts_detect_tampering() {
    let (_dir, mut p, _) = fixture();
    let locale: LocaleId = "fr".parse().unwrap();
    let mut rows = decode(&p.locale_export(&locale, false).unwrap(), false).unwrap();
    rows[0].forms.insert(PluralCategory::Other, "Bonjour {name}.".into());
    let mut unknown = rows[0].clone();
    unknown.source.id = wobu_core::new_id().to_string();
    rows.push(unknown);
    let report = p.locale_import(&encode(&rows, false).unwrap(), false).unwrap();
    assert_eq!(report.applied.len(), 1);
    assert!(report.diagnostics.iter().any(|d| d.code == "unknown_id"));
    let mut files = p.narrative_records(wobu_store::NarrativeRecordKind::Production).unwrap();
    let file = &mut files[0];
    file.document.payload["translation"]["history"][0]["approved"] = true.into();
    p.save_narrative_record(file).unwrap();
    assert!(p.locale_translations().unwrap_err().to_string().contains("immutable"));
}
#[test]
fn strict_release_and_explicit_source_fallback_use_configured_policy() {
    let (_dir, mut p, _) = fixture();
    let (mut policy, guard) = p.locale_policy().unwrap();
    policy.required = BTreeMap::from([("fr-CA".parse().unwrap(), false)]);
    p.save_locale_policy(policy.clone(), &guard).unwrap();
    assert!(p.locale_release().unwrap().1.iter().any(|d| d.code == "missing_translation"));
    let (_, guard) = p.locale_policy().unwrap();
    policy.required.insert("fr-CA".parse().unwrap(), true);
    p.save_locale_policy(policy, &guard).unwrap();
    let (bundle, diagnostics) = p.locale_release().unwrap();
    assert!(diagnostics.iter().all(|d| d.code == "fallback"));
    assert!(!bundle.strings.is_empty());
}

#[test]
fn two_open_projects_preserve_the_winning_translation_and_report_the_old_guard() {
    let (_dir, mut first, _) = fixture();
    let locale: LocaleId = "fr".parse().unwrap();
    let mut rows = decode(&first.locale_export(&locale, false).unwrap(), false).unwrap();
    rows[0].forms.insert(PluralCategory::Other, "Bonjour {name}.".into());
    let mut peer = Project::open(first.root()).unwrap();
    assert_eq!(peer.locale_import(&encode(&rows, false).unwrap(), false).unwrap().applied.len(), 1);
    rows[0].forms.insert(PluralCategory::Other, "Bonsoir {name}.".into());
    let report = first.locale_import(&encode(&rows, false).unwrap(), false).unwrap();
    assert!(report.applied.is_empty());
    assert!(report.diagnostics.iter().any(|d| d.code == "translation_conflict"));
    assert_eq!(
        first.locale_translations().unwrap().values().next().unwrap().latest().forms
            [&PluralCategory::Other],
        "Bonjour {name}."
    );
}

#[test]
fn batch_review_capture_preserves_single_target_proofs_and_observes_external_source_edits() {
    let (_dir, project, id) = fixture();
    let scene = SceneId::from_raw(id.raw());
    let batch = project.review_snapshots(&[scene], None).unwrap();
    let single = project.review_snapshot(scene, None).unwrap();
    assert_eq!(batch[0].text_evidence().unwrap(), single.text_evidence().unwrap());
    assert_eq!(batch[0].guard(), single.guard());
    let file = project.load_text_asset(id).unwrap();
    let path = project.root().join(file.rel);
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("\n# external collaborator\n");
    std::fs::write(&path, text).unwrap();
    assert!(batch[0].verify_current(&project).is_err());
}
