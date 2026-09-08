use std::collections::BTreeMap;
use wobu_narrative_locale::{
    format::placeholders,
    interchange::{decode, encode},
    *,
};
fn source() -> SourceLine {
    SourceLine {
        id: wobu_core::Id::from(1u128).to_string(),
        slot: "slot".into(),
        container: "scene".into(),
        speaker: "Mara".into(),
        text: "Hello, \"{name}\"!\nYou have {n:number} lights. {{literal}}".into(),
        revision: "revision".into(),
        guard: "guard".into(),
        context: "Harbor\nAt dusk".into(),
        delivery_notes: "Warm, quiet".into(),
        placeholders: placeholders("{name} {n:number}"),
        ready: true,
    }
}
fn row() -> Row {
    Row {
        version: VERSION,
        locale: LocaleId::parse("ar").unwrap(),
        source: source(),
        translation_guard: None,
        forms: BTreeMap::from([(
            PluralCategory::Other,
            "مرحباً \"{name}\"!\n{n:number} أضواء {{literal}}".into(),
        )]),
    }
}
#[test]
fn csv_and_json_preserve_quotes_newlines_unicode_rtl_and_stable_metadata() {
    for csv in [true, false] {
        let original = vec![row()];
        let encoded = encode(&original, csv).unwrap();
        assert_eq!(decode(&encoded, csv).unwrap(), original);
        assert_eq!(encode(&decode(&encoded, csv).unwrap(), csv).unwrap(), encoded);
    }
}
#[test]
fn per_row_diagnostics_refuse_stale_duplicate_unknown_missing_and_placeholders() {
    let original = row();
    let mut sources = BTreeMap::from([(original.source.id.clone(), original.source.clone())]);
    sources.insert("missing".into(), SourceLine { id: "missing".into(), ..source() });
    let mut stale = original.clone();
    stale.source.guard = "old".into();
    stale.forms.insert(PluralCategory::Other, "بدون الاسم".into());
    let mut unknown = original.clone();
    unknown.source.id = "unknown".into();
    let diagnostics = preview(&[stale, original, unknown], &sources, &BTreeMap::new());
    for code in ["stale_source", "duplicate_id", "unknown_id", "missing_id", "placeholders"] {
        assert!(diagnostics.iter().any(|d| d.code == code), "{code}");
    }
}
#[test]
fn import_rejects_mixed_locales_and_future_versions_are_per_row() {
    let mut other = row();
    other.locale = "fr".parse().unwrap();
    assert!(
        decode(&encode(&[row(), other], false).unwrap(), false)
            .unwrap_err()
            .to_string()
            .contains("one target locale")
    );
    let mut future = row();
    future.version = 9;
    assert!(
        preview(&[future], &BTreeMap::from([(source().id.clone(), source())]), &BTreeMap::new())
            .iter()
            .any(|d| d.code == "version")
    );
}
#[test]
fn fallback_requires_explicit_policy_and_plural_render_is_plain_deterministic_text() {
    let source = source();
    let sources = BTreeMap::from([(source.id.clone(), source.clone())]);
    let target: LocaleId = "ar-EG".parse().unwrap();
    let mut policy = Policy::default();
    policy.required.insert(target.clone(), false);
    let (_, diagnostics) = release::prepare(&policy, &sources, &BTreeMap::new());
    assert_eq!(diagnostics[0].code, "missing_translation");
    policy.required.insert(target.clone(), true);
    let (fallback, diagnostics) = release::prepare(&policy, &sources, &BTreeMap::new());
    assert_eq!(diagnostics[0].code, "fallback");
    assert_eq!(
        fallback.text(&target, &source.id, PluralCategory::Many),
        Some(source.text.as_str())
    );
    let mut translation = Translation {
        version: VERSION,
        variant_id: source.id.clone(),
        locale: "ar".parse().unwrap(),
        history: vec![TranslationVersion {
            source_revision: source.revision.clone(),
            source_guard: source.guard.clone(),
            forms: row().forms,
            approved: true,
            actor: "reviewer".into(),
        }],
    };
    translation.latest();
    let translations =
        BTreeMap::from([((translation.locale.clone(), source.id.clone()), translation.clone())]);
    let (bundle, _) = release::prepare(&policy, &sources, &translations);
    let rendered = bundle
        .render(
            &target,
            &source.id,
            PluralCategory::Few,
            &BTreeMap::from([
                ("name".into(), "<script>{n:number}</script>".into()),
                ("n:number".into(), "٣".into()),
            ]),
        )
        .unwrap();
    assert!(rendered.contains("<script>{n:number}</script>"));
    assert!(rendered.contains("٣"));
    assert!(rendered.contains("{literal}"));
    translation.history[0].source_guard = "before unlock".into();
    assert!(!translation.current(&source));
}
