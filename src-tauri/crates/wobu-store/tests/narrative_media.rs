use std::collections::BTreeMap;
use wobu_narrative::{
    DialogueSlot, GenerationPolicy, Name, SceneId, Speaker, Text, TextEntry, TextKind, Variant,
    review::{EditorialAction, PolicyScope},
};
use wobu_narrative_locale::{LocaleId, PluralCategory};
use wobu_narrative_media::{
    self as media, Row,
    interchange::{decode, encode},
};
use wobu_store::{Project, SourceSave, project::narrative_review::ReviewRequest};
#[path = "../../wobu-narrative-media/tests/support/mod.rs"]
mod support;
fn fixture() -> (tempfile::TempDir, Project, wobu_narrative::TextAssetId) {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::create(dir.path(), "Recording world").unwrap();
    let mut file =
        p.create_text_asset(TextKind::Codex, "Harbor", Name::new("read").unwrap()).unwrap();
    let mut entry = TextEntry::new("Arrival");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written("Hello, \"{name}\"!\nAt the harbor.")));
    entry.lines.push(slot);
    file.asset.entries.push(entry);
    p.save_text_asset(&mut file).unwrap();
    let id = file.asset.id;
    action(&mut p, id, EditorialAction::Approve);
    action(
        &mut p,
        id,
        EditorialAction::Policy { scope: PolicyScope::Slot, policy: GenerationPolicy::Locked },
    );
    (dir, p, id)
}
fn action(p: &mut Project, id: wobu_narrative::TextAssetId, action: EditorialAction) {
    let view = p.review_scene(SceneId::from_raw(id.raw()), None).unwrap();
    let line = &view.lines[0];
    p.apply_review(&ReviewRequest {
        guard: view.guard.clone(),
        target: line.target.clone(),
        context_revision: line.context_revision.clone(),
        state_json: view.state_json.clone(),
        action,
    })
    .unwrap();
}
fn prepared(p: &Project, root: &std::path::Path, locale: &LocaleId) -> Row {
    let mut row = decode(&p.media_export(locale, false).unwrap(), false).unwrap().remove(0);
    row.parameters.insert("name".into(), "Mira".into());
    let wav = support::wav();
    let path = root.join(&row.audio_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &wav).unwrap();
    let track = media::timing::Track {
        version: 1,
        audio_hash: blake3::hash(&wav).to_hex().to_string(),
        duration_ms: 1000,
        cues: vec![media::timing::Cue {
            start_ms: 0,
            end_ms: 800,
            kind: media::timing::Kind::Word,
            value: "Hello".into(),
        }],
    };
    let timing = row.audio_path.replace(".wav", ".timing.json");
    std::fs::write(root.join(&timing), serde_json::to_vec(&track).unwrap()).unwrap();
    row.timing_path = Some(timing);
    row
}
#[test]
fn real_import_audition_history_unchanged_id_edit_and_reopen() {
    let (dir, mut p, id) = fixture();
    let locale = "en".parse().unwrap();
    let row = prepared(&p, dir.path(), &locale);
    let input = encode(std::slice::from_ref(&row), true).unwrap();
    assert!(p.media_preview(&input, true, dir.path()).unwrap().is_empty());
    assert_eq!(p.media_import(&input, true, dir.path()).unwrap().applied.len(), 1);
    let binding = p.media_bindings().unwrap()[&row.key.token()].clone();
    let take = binding.latest().unwrap();
    assert_eq!(take.spoken_text, "Hello, \"Mira\"!\nAt the harbor.");
    assert_eq!(p.media_sync_blobs().unwrap().len(), 2);
    let audition = p.media_audition(&row.key, 0).unwrap();
    assert!(audition.current);
    assert_eq!(audition.timing.unwrap().active(200).count(), 1);
    assert!(std::path::Path::new(&audition.path).is_file());
    assert!(p.media_import(&input, true, dir.path()).unwrap().applied.is_empty());
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
    assert!(!p.media_audition(&row.key, 0).unwrap().current);
    action(
        &mut p,
        id,
        EditorialAction::Policy { scope: PolicyScope::Slot, policy: GenerationPolicy::Edited },
    );
    action(&mut p, id, EditorialAction::Edit { body: "Hello {name}. The watch continues.".into() });
    assert_eq!(p.media_bindings().unwrap()[&row.key.token()], binding);
    assert!(
        p.media_view(&locale)
            .unwrap()
            .rows
            .iter()
            .any(|r| r.key.id == row.key.id && r.source.revision != row.source.revision)
    );
    let root = p.root().to_owned();
    let index = p.index_path().to_owned();
    drop(p);
    std::fs::remove_file(index).unwrap();
    let reopened = Project::open(&root).unwrap();
    assert_eq!(reopened.media_bindings().unwrap()[&row.key.token()], binding);
    assert_eq!(reopened.media_sync_blobs().unwrap().len(), 2);
}
#[test]
fn partial_validation_paths_malformed_timing_duplicates_and_competing_import() {
    let (dir, mut p, _) = fixture();
    let locale = "en".parse().unwrap();
    let row = prepared(&p, dir.path(), &locale);
    for path in ["../escape.wav", "/absolute.wav", "dir\\take.wav", "NUL.wav"] {
        let mut invalid = row.clone();
        invalid.audio_path = path.into();
        let report =
            p.media_import(&encode(&[invalid], false).unwrap(), false, dir.path()).unwrap();
        assert!(report.applied.is_empty());
        assert!(!report.diagnostics.is_empty());
    }
    let mut unknown = row.clone();
    unknown.key.id = wobu_core::new_id().to_string();
    let input = encode(&[row.clone(), unknown], false).unwrap();
    let report = p.media_import(&input, false, dir.path()).unwrap();
    assert_eq!(report.applied.len(), 1);
    assert!(report.diagnostics.iter().any(|d| d.code == "unknown"));
    let next = prepared(&p, dir.path(), &locale);
    let duplicate = encode(&[next.clone(), next.clone()], false).unwrap();
    assert!(p.media_import(&duplicate, false, dir.path()).unwrap().applied.is_empty());
    let mut peer = Project::open(p.root()).unwrap();
    assert_eq!(
        peer.media_import(&encode(std::slice::from_ref(&next), false).unwrap(), false, dir.path())
            .unwrap()
            .applied
            .len(),
        1
    );
    assert!(
        p.media_import(&encode(&[next], false).unwrap(), false, dir.path())
            .unwrap()
            .diagnostics
            .iter()
            .any(|d| d.code == "media_conflict")
    );
    let bad = prepared(&p, dir.path(), &locale);
    std::fs::write(dir.path().join(bad.timing_path.as_ref().unwrap()), b"{\"version\":999}")
        .unwrap();
    assert!(
        p.media_preview(&encode(&[bad], false).unwrap(), false, dir.path())
            .unwrap()
            .iter()
            .any(|d| d.code == "invalid_media")
    );
}
#[test]
fn translated_takes_bind_exact_approval_and_required_media_has_explicit_fallback() {
    let (dir, mut p, _) = fixture();
    let locale: LocaleId = "ar".parse().unwrap();
    let mut translations = wobu_narrative_locale::interchange::decode(
        &p.locale_export(&locale, false).unwrap(),
        false,
    )
    .unwrap();
    translations[0].forms.insert(PluralCategory::Other, "مرحباً {name}.".into());
    p.locale_import(
        &wobu_narrative_locale::interchange::encode(&translations, false).unwrap(),
        false,
    )
    .unwrap();
    assert!(p.media_export(&locale, false).unwrap().contains("[]"));
    let approved = wobu_narrative_locale::interchange::decode(
        &p.locale_export(&locale, false).unwrap(),
        false,
    )
    .unwrap();
    assert!(matches!(p.locale_approve(&approved[0]).unwrap(), SourceSave::Saved(_)));
    let (mut locale_policy, guard) = p.locale_policy().unwrap();
    locale_policy.required.insert(locale.clone(), false);
    p.save_locale_policy(locale_policy, &guard).unwrap();
    let (mut policy, guard) = p.media_policy().unwrap();
    policy.required.insert(locale.clone(), false);
    p.save_media_policy(policy.clone(), &guard).unwrap();
    assert!(p.media_release().unwrap().diagnostics.iter().any(|d| d.code == "missing_media"));
    let row = prepared(&p, dir.path(), &locale);
    assert_eq!(
        p.media_import(&encode(std::slice::from_ref(&row), false).unwrap(), false, dir.path())
            .unwrap()
            .applied
            .len(),
        1
    );
    let release = p.media_release().unwrap();
    assert!(release.diagnostics.iter().any(|d| d.code == "missing_media"));
    let (_, guard) = p.media_policy().unwrap();
    policy.required.insert(locale.clone(), true);
    p.save_media_policy(policy.clone(), &guard).unwrap();
    let release = p.media_release().unwrap();
    assert!(release.diagnostics.is_empty());
    assert_eq!(release.bundle.lookup(&row.key, &row.parameters).unwrap().origin, locale);
    assert!(release.bundle.lookup(&row.key, &BTreeMap::new()).is_none());
    let mut updated = wobu_narrative_locale::interchange::decode(
        &p.locale_export(&locale, false).unwrap(),
        false,
    )
    .unwrap();
    updated[0].forms.insert(PluralCategory::Other, "أهلاً {name}.".into());
    p.locale_import(&wobu_narrative_locale::interchange::encode(&updated, false).unwrap(), false)
        .unwrap();
    assert!(p.media_release().unwrap().diagnostics.iter().any(|d| d.code == "text_fallback"));
    let (_, guard) = p.media_policy().unwrap();
    policy.required.insert(locale, true);
    p.save_media_policy(policy, &guard).unwrap();
    assert!(p.media_release().unwrap().diagnostics.iter().all(|d| d.code == "text_fallback"));
}
#[test]
fn incomplete_blob_publication_cannot_create_a_valid_link_and_tampering_is_visible() {
    let (dir, mut p, _) = fixture();
    let locale = "en".parse().unwrap();
    let row = prepared(&p, dir.path(), &locale);
    let hash = blake3::hash(&support::wav()).to_hex().to_string();
    let path = p.root().join(format!("assets/media/{hash}.wav"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"interrupted").unwrap();
    assert!(
        p.media_import(&encode(std::slice::from_ref(&row), false).unwrap(), false, dir.path())
            .unwrap()
            .applied
            .is_empty()
    );
    assert!(p.media_bindings().unwrap().is_empty());
    std::fs::remove_file(&path).unwrap();
    p.media_import(&encode(std::slice::from_ref(&row), false).unwrap(), false, dir.path()).unwrap();
    std::fs::write(&path, b"tampered").unwrap();
    assert!(p.media_audition(&row.key, 0).is_err());
    assert_eq!(p.media_view(&locale).unwrap().diagnostics.len(), 1);
}
#[cfg(unix)]
#[test]
fn symlinked_inputs_are_refused_without_reading_the_external_file() {
    let (dir, mut p, _) = fixture();
    let row = prepared(&p, dir.path(), &"en".parse().unwrap());
    let path = dir.path().join(&row.audio_path);
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", &path).unwrap();
    assert!(
        p.media_import(&encode(&[row], false).unwrap(), false, dir.path())
            .unwrap()
            .applied
            .is_empty()
    );
}

#[test]
fn required_timing_blocks_an_audio_only_static_take_until_explicit_fallback() {
    let (dir, mut p, id) = fixture();
    action(
        &mut p,
        id,
        EditorialAction::Policy { scope: PolicyScope::Slot, policy: GenerationPolicy::Edited },
    );
    action(&mut p, id, EditorialAction::Edit { body: "The harbor is quiet.".into() });
    action(&mut p, id, EditorialAction::Approve);
    action(
        &mut p,
        id,
        EditorialAction::Policy { scope: PolicyScope::Slot, policy: GenerationPolicy::Locked },
    );
    let locale: LocaleId = "en".parse().unwrap();
    let mut row = prepared(&p, dir.path(), &locale);
    row.parameters.clear();
    row.timing_path = None;
    p.media_import(&encode(std::slice::from_ref(&row), false).unwrap(), false, dir.path()).unwrap();
    let (mut policy, guard) = p.media_policy().unwrap();
    policy.required.insert(locale.clone(), false);
    policy.timing.insert(locale.clone());
    p.save_media_policy(policy.clone(), &guard).unwrap();
    let release = p.media_release().unwrap();
    assert!(release.bundle.takes.is_empty());
    assert!(release.diagnostics.iter().any(|d| d.code == "missing_media"));
    let (_, guard) = p.media_policy().unwrap();
    policy.required.insert(locale, true);
    p.save_media_policy(policy, &guard).unwrap();
    let release = p.media_release().unwrap();
    assert!(release.bundle.fallback.contains(&row.key.token()));
    assert!(release.diagnostics.iter().all(|d| d.code == "text_fallback"));
}
#[test]
fn list_and_sync_do_not_read_unchanged_size_clip_content_but_audition_refuses_it() {
    let (dir, mut p, _) = fixture();
    let locale = "en".parse().unwrap();
    let row = prepared(&p, dir.path(), &locale);
    p.media_import(&encode(std::slice::from_ref(&row), false).unwrap(), false, dir.path()).unwrap();
    let binding = p.media_bindings().unwrap()[&row.key.token()].clone();
    let blob = &binding.latest().unwrap().audio;
    let path = p.root().join(&blob.path);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[44] ^= 1;
    std::fs::write(path, bytes).unwrap();
    assert!(p.media_view(&locale).unwrap().diagnostics.is_empty());
    assert_eq!(p.media_sync_blobs().unwrap().len(), 2);
    assert!(p.media_audition(&row.key, 0).is_err());
}
