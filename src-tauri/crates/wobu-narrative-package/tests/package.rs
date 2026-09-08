use std::path::PathBuf;
use wobu_narrative::*;
use wobu_narrative_compiler::{CompileOptions, Graph, Profile, compile};
use wobu_narrative_package::{Manifest, Package, publish, read};

#[path = "../../wobu-narrative-compiler/tests/support/reviews.rs"]
mod reviews;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "wobu-package-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(profile: Profile) -> Graph {
    let mut scene = Scene::new("Author-only secret name");
    scene.id = "00000000000000000000000001".parse().unwrap();
    scene.summary = "secret prompt".into();
    let mut beat = Beat::new("Author-only intent");
    beat.id = "00000000000000000000000002".parse().unwrap();
    beat.must_not_reveal = vec!["secret future".into()];
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.id = "00000000000000000000000003".parse().unwrap();
    let mut variant = Variant::new(Text::written("星の港へ — café"));
    variant.id = "00000000000000000000000004".parse().unwrap();
    variant.text.lifecycle.review = ReviewState::Approved;
    slot.variants.push(variant);
    beat.dialogue.push(slot);
    let mut choice = Choice::new("続ける", Destination::End { label: "done".into() });
    choice.id = "00000000000000000000000005".parse().unwrap();
    beat.choices.push(choice);
    scene.beats.push(beat);
    let verified_reviews = reviews::fixture_reviews(&scene);
    compile(
        &[scene],
        &[],
        &StateSchema::default(),
        &CompileOptions { profile, verified_reviews, ..CompileOptions::default() },
    )
    .graph
    .unwrap()
}
fn edit_manifest(root: &std::path::Path, edit: impl FnOnce(&mut Manifest)) {
    let path = root.join("manifest.json");
    let mut manifest: Manifest = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    edit(&mut manifest);
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
}
fn edit_payload(root: &std::path::Path, name: &str, bytes: &[u8]) {
    std::fs::write(root.join(name), bytes).unwrap();
    edit_manifest(root, |m| {
        let entry = m.files.get_mut(name).unwrap();
        entry.bytes = bytes.len() as u64;
        entry.hash = blake3::hash(bytes).to_hex().to_string();
        m.payload_hash = blake3::hash(&serde_json::to_vec(&m.files).unwrap()).to_hex().to_string();
    });
}
#[test]
fn deterministic_unicode_golden_roundtrip_contains_only_runtime_assets() {
    let original = fixture(Profile::Release);
    let package = Package::build(original.clone(), false).unwrap();
    assert_eq!(
        package.manifest_bytes().unwrap(),
        Package::build(fixture(Profile::Release), false).unwrap().manifest_bytes().unwrap()
    );
    let temp = Temp::new();
    let out = temp.0.join("package");
    publish(&package, &out).unwrap();
    let loaded = read(&out).unwrap();
    let mut expected = original;
    expected.source_map.clear();
    assert_eq!(loaded.graph().unwrap(), expected);
    assert_eq!(
        package.manifest_bytes().unwrap(),
        include_bytes!("fixtures/manifest.json").to_vec()
    );
    for file in package.manifest.files.keys() {
        let text = std::fs::read_to_string(out.join(file)).unwrap();
        for excluded in ["secret", "Author-only", "provenance", "lifecycle", "api_key"] {
            assert!(!text.contains(excluded), "{file}: {excluded}");
        }
    }
    assert_eq!(package.string_count().unwrap(), 2);
    assert!(!out.join("debug").exists());
    assert!(publish(&package, &out).is_err());
    assert_eq!(read(&out).unwrap().manifest, package.manifest);
}
#[test]
fn corrupt_missing_and_duplicate_strings_fail_even_with_updated_hashes() {
    for replacement in
        [b"{}".as_slice(), b"{\"same\":{},\"same\":{}}".as_slice(), b"null".as_slice()]
    {
        let temp = Temp::new();
        let out = temp.0.join("package");
        publish(&Package::build(fixture(Profile::Release), false).unwrap(), &out).unwrap();
        edit_payload(&out, "strings/en.json", replacement);
        assert!(read(&out).is_err());
    }
    let temp = Temp::new();
    let out = temp.0.join("package");
    publish(&Package::build(fixture(Profile::Release), false).unwrap(), &out).unwrap();
    std::fs::write(out.join("graph.json"), b"{}").unwrap();
    assert!(read(&out).is_err());
}
#[test]
fn unsupported_versions_capabilities_paths_and_sizes_are_rejected() {
    type ManifestMutation = Box<dyn Fn(&mut Manifest)>;
    let cases: Vec<ManifestMutation> = vec![
        Box::new(|m| m.version = 999),
        Box::new(|m| {
            m.required_capabilities.insert("execute_script".into(), 1);
        }),
        Box::new(|m| {
            let record = m.files.remove("graph.json").unwrap();
            m.files.insert("../graph.json".into(), record);
        }),
        Box::new(|m| {
            m.files.get_mut("graph.json").unwrap().bytes = u64::MAX;
        }),
        Box::new(|m| {
            m.files.get_mut("graph.json").unwrap().hash = "a".repeat(64);
        }),
    ];
    for edit in cases {
        let temp = Temp::new();
        let out = temp.0.join("package");
        publish(&Package::build(fixture(Profile::Release), false).unwrap(), &out).unwrap();
        edit_manifest(&out, edit);
        assert!(read(&out).is_err());
    }
}
#[test]
fn incomplete_and_absent_manifest_are_never_loadable() {
    let temp = Temp::new();
    let out = temp.0.join("package");
    publish(&Package::build(fixture(Profile::Release), false).unwrap(), &out).unwrap();
    std::fs::write(out.join(".incomplete"), b"interrupted").unwrap();
    assert!(read(&out).unwrap_err().to_string().contains("incomplete"));
    std::fs::remove_file(out.join(".incomplete")).unwrap();
    std::fs::remove_file(out.join("manifest.json")).unwrap();
    assert!(read(&out).is_err());
}
#[test]
fn development_debug_maps_are_optional_and_release_refuses_blank_choices() {
    let graph = fixture(Profile::Development);
    let package = Package::build(graph.clone(), true).unwrap();
    assert_eq!(package.graph().unwrap(), graph);
    assert!(Package::build(fixture(Profile::Release), true).is_err());
    let mut graph = fixture(Profile::Release);
    graph.scenes.values_mut().next().unwrap().beats.values_mut().next().unwrap().choices[0].label =
        "  ".into();
    assert!(Package::build(graph, false).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_payload_and_ancestor_are_rejected() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let out = temp.0.join("package");
    publish(&Package::build(fixture(Profile::Release), false).unwrap(), &out).unwrap();
    symlink(&out, temp.0.join("alias")).unwrap();
    assert!(read(&temp.0.join("alias")).is_err());
    assert!(
        publish(
            &Package::build(fixture(Profile::Release), false).unwrap(),
            &temp.0.join("alias/new")
        )
        .is_err()
    );
    std::fs::rename(out.join("state.json"), temp.0.join("state.json")).unwrap();
    symlink(temp.0.join("state.json"), out.join("state.json")).unwrap();
    assert!(read(&out).is_err());
}

#[test]
fn malformed_graph_references_and_id_aliases_fail_validation() {
    let mut graph = fixture(Profile::Development);
    graph.source_map.clear();
    graph.scenes.values_mut().next().unwrap().beats.values_mut().next().unwrap().dialogue[0]
        .variants[0]
        .id = "0000000000000000000000000a".into();
    assert!(Package::build(graph, false).unwrap_err().to_string().contains("non-canonical"));
    let temp = Temp::new();
    let out = temp.0.join("package");
    publish(&Package::build(fixture(Profile::Development), false).unwrap(), &out).unwrap();
    let mut graph: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join("graph.json")).unwrap()).unwrap();
    graph["scenes"]["00000000000000000000000001"]["first"] =
        serde_json::json!("00000000000000000000000009");
    edit_payload(&out, "graph.json", &serde_json::to_vec(&graph).unwrap());
    assert!(read(&out).unwrap_err().to_string().contains("missing scene entry"));
}

#[test]
fn publication_claim_is_exclusive_under_concurrent_writers() {
    let temp = Temp::new();
    let out = temp.0.join("package");
    let package = std::sync::Arc::new(Package::build(fixture(Profile::Release), false).unwrap());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let package = package.clone();
            let barrier = barrier.clone();
            let out = out.clone();
            std::thread::spawn(move || {
                barrier.wait();
                publish(&package, &out)
            })
        })
        .collect();
    let successes = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .filter(|result| result.is_ok())
        .count();
    assert_eq!(successes, 1);
    assert_eq!(read(&out).unwrap().manifest, package.manifest);
}

/// The same fixture plus one supporting text asset of each spoken and written
/// kind (#167), so the packaged form can be checked for the two things that
/// matter: the strings come out into the shared table, and the manifest says so.
fn text_fixture(profile: Profile) -> Graph {
    let mut scene = Scene::new("Author-only secret name");
    scene.id = "00000000000000000000000001".parse().unwrap();
    let mut beat = Beat::new("Author-only intent");
    beat.id = "00000000000000000000000002".parse().unwrap();
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.id = "00000000000000000000000003".parse().unwrap();
    let mut variant = Variant::new(Text::written("星の港へ — café"));
    variant.id = "00000000000000000000000004".parse().unwrap();
    slot.variants.push(variant);
    beat.dialogue.push(slot);
    beat.outcomes.push(Outcome::new(Destination::End { label: "done".into() }));
    scene.beats.push(beat);

    let entity: EntityId = "00000000000000000000000010".parse().unwrap();
    let mut bark =
        TextAsset::new(TextKind::Bark, "Gate guard", Name::new("player_passes_gate").unwrap());
    bark.id = "00000000000000000000000011".parse().unwrap();
    bark.participants.push(Participant { entity, role: String::new() });
    let mut entry = TextEntry::new("Move along");
    entry.id = "00000000000000000000000012".parse().unwrap();
    let mut line = DialogueSlot::new(Speaker::Entity(entity));
    line.id = "00000000000000000000000013".parse().unwrap();
    let mut wording = Variant::new(Text::written("動け。"));
    wording.id = "00000000000000000000000014".parse().unwrap();
    line.variants.push(wording);
    entry.lines.push(line);
    bark.entries.push(entry);

    let mut codex =
        TextAsset::new(TextKind::Codex, "The beacons", Name::new("codex_opened").unwrap());
    codex.id = "00000000000000000000000021".parse().unwrap();
    let mut page = TextEntry::new("Overview");
    page.id = "00000000000000000000000022".parse().unwrap();
    let mut prose = DialogueSlot::new(Speaker::Narrator);
    prose.id = "00000000000000000000000023".parse().unwrap();
    let mut body = Variant::new(Text::written("The beacons predate the harbour."));
    body.id = "00000000000000000000000024".parse().unwrap();
    prose.variants.push(body);
    page.lines.push(prose);
    codex.entries.push(page);

    let texts = [bark, codex];
    let verified_reviews = reviews::fixture_reviews(&scene);
    let verified_text_reviews = texts.iter().flat_map(reviews::fixture_text_reviews).collect();
    compile(
        &[scene],
        &texts,
        &StateSchema::default(),
        &CompileOptions {
            profile,
            verified_reviews,
            verified_text_reviews,
            known_entities: std::collections::BTreeSet::from([entity]),
            ..CompileOptions::default()
        },
    )
    .graph
    .unwrap()
}

#[test]
fn supporting_text_is_packaged_into_the_shared_string_table_and_declared() {
    let original = text_fixture(Profile::Release);
    let package = Package::build(original.clone(), false).unwrap();

    assert_eq!(
        package.manifest.required_capabilities.get("supporting_text"),
        Some(&1),
        "a reader that cannot deliver barks must be told to refuse this package"
    );
    // Two scene strings — one line, one choice label — plus one bark and one
    // codex line, all keyed by the identity a locale row and a recording script
    // already use.
    assert_eq!(package.string_count().unwrap(), 3);

    let temp = Temp::new();
    let out = temp.0.join("package");
    publish(&package, &out).unwrap();
    let mut expected = original;
    expected.source_map.clear();
    assert_eq!(read(&out).unwrap().graph().unwrap(), expected);

    let strings = std::fs::read_to_string(out.join("strings/en.json")).unwrap();
    assert!(strings.contains("動け。"), "{strings}");
    let graph = std::fs::read_to_string(out.join("graph.json")).unwrap();
    assert!(!graph.contains("動け。"), "wording must not be inlined in the graph");
}

#[test]
fn a_package_without_supporting_text_declares_exactly_the_capabilities_it_always_did() {
    let package = Package::build(fixture(Profile::Release), false).unwrap();
    assert!(!package.manifest.required_capabilities.contains_key("supporting_text"));
}

#[test]
fn a_manifest_that_disagrees_with_its_payload_about_supporting_text_is_refused() {
    let temp = Temp::new();
    let out = temp.0.join("package");
    publish(&Package::build(text_fixture(Profile::Release), false).unwrap(), &out).unwrap();
    // Removing the declaration leaves a well formed, correctly hashed package
    // that would silently drop every bark. Only the cross-check catches it.
    edit_manifest(&out, |m| {
        m.required_capabilities.remove("supporting_text");
        m.payload_hash = blake3::hash(&serde_json::to_vec(&m.files).unwrap()).to_hex().to_string();
    });
    // `read` reconstructs the graph, so the disagreement is caught before a
    // caller can ever hold the package.
    let error = read(&out).unwrap_err().to_string();
    assert!(error.contains("capabilities"), "{error}");
}

#[test]
fn release_packaging_still_refuses_empty_supporting_wording() {
    let mut graph = text_fixture(Profile::Release);
    let asset = graph.texts.values_mut().next().unwrap();
    asset.entries[0].lines[0].variants[0].text = "   ".into();
    let error = Package::build(graph, false).unwrap_err().to_string();
    assert!(error.contains("empty"), "{error}");
}

#[test]
fn native_locale_payload_roundtrips_rtl_and_explicit_fallback_without_changing_source() {
    use std::collections::BTreeMap;
    use wobu_narrative_locale::{
        PluralCategory, Policy,
        release::{Bundle, Localized},
    };
    let mut graph = fixture(Profile::Release);
    graph.source_map.clear();
    let original = Package::build(graph.clone(), false).unwrap();
    let locale: wobu_narrative_locale::LocaleId = "ar".parse().unwrap();
    let bundle = Bundle {
        version: 1,
        policy: Policy { required: BTreeMap::from([(locale.clone(), true)]), ..Policy::default() },
        strings: BTreeMap::from([(
            locale.clone(),
            BTreeMap::from([
                (
                    "00000000000000000000000004".into(),
                    Localized {
                        locale: locale.clone(),
                        forms: BTreeMap::from([(
                            PluralCategory::Other,
                            "ميناء النجوم\nمرحبا".into(),
                        )]),
                    },
                ),
                (
                    "00000000000000000000000005".into(),
                    Localized {
                        locale: "en".parse().unwrap(),
                        forms: BTreeMap::from([(PluralCategory::Other, "続ける".into())]),
                    },
                ),
            ]),
        )]),
    };
    let package = original.with_locales(bundle.clone()).unwrap();
    assert_eq!(package.manifest.required_capabilities["localisation"], 1);
    let temp = Temp::new();
    let out = temp.0.join("localised");
    publish(&package, &out).unwrap();
    let reopened = read(&out).unwrap();
    assert_eq!(reopened.graph().unwrap(), graph);
    assert_eq!(reopened.locales().unwrap(), Some(bundle));
    let translated = reopened.graph_locale(&locale).unwrap();
    let beat = translated.scenes.values().next().unwrap().beats.values().next().unwrap();
    assert_eq!(beat.dialogue[0].variants[0].text, "ميناء النجوم\nمرحبا");
    assert_eq!(beat.choices[0].label, "続ける");
}

#[path = "../../wobu-narrative-media/tests/support/mod.rs"]
mod media_support;
#[test]
fn prepared_media_package_roundtrip_native_timing_and_fallback_validation() {
    use std::collections::{BTreeMap, BTreeSet};
    use wobu_narrative_media::{
        self as media,
        release::{Bundle, Prepared},
    };
    let graph = fixture(Profile::Release);
    let variant = &graph.scenes.values().next().unwrap().beats.values().next().unwrap().dialogue[0]
        .variants[0];
    let key = media::Key {
        id: variant.id.clone(),
        locale: "en".parse().unwrap(),
        form: wobu_narrative_locale::PluralCategory::Other,
    };
    let audio = media_support::wav();
    let hash = blake3::hash(&audio).to_hex().to_string();
    let track = media::timing::Track {
        version: 1,
        audio_hash: hash.clone(),
        duration_ms: 1000,
        cues: vec![media::timing::Cue {
            start_ms: 0,
            end_ms: 800,
            kind: media::timing::Kind::Viseme,
            value: "aa".into(),
        }],
    };
    let timing = serde_json::to_vec(&track).unwrap();
    let timing_hash = blake3::hash(&timing).to_hex().to_string();
    let take = Prepared {
        key: key.clone(),
        origin: key.locale.clone(),
        source_revision: variant.revision.clone(),
        translation_revision: None,
        template: variant.text.clone(),
        spoken_text: variant.text.clone(),
        parameters: BTreeMap::new(),
        audio: media::Blob {
            path: format!("assets/media/{hash}.wav"),
            hash,
            bytes: audio.len() as u64,
        },
        info: media::wav::inspect(&audio).unwrap(),
        timing: Some(media::Blob {
            path: format!("assets/media/{timing_hash}.json"),
            hash: timing_hash,
            bytes: timing.len() as u64,
        }),
    };
    let fallback = media::Key { id: "00000000000000000000000005".into(), ..key.clone() }.token();
    let bundle = Bundle {
        version: 1,
        required: BTreeMap::from([(key.locale.clone(), true)]),
        timing: BTreeSet::from([key.locale.clone()]),
        takes: BTreeMap::from([(key.token(), take.clone())]),
        fallback: BTreeSet::from([fallback]),
    };
    let files = BTreeMap::from([
        (take.audio.path.clone(), audio.clone()),
        (take.timing.as_ref().unwrap().path.clone(), timing),
    ]);
    let package = Package::build(graph.clone(), false)
        .unwrap()
        .with_media(bundle.clone(), files.clone())
        .unwrap();
    let temp = Temp::new();
    publish(&package, &temp.0.join("voiced")).unwrap();
    let reopened = read(&temp.0.join("voiced")).unwrap();
    let media = reopened.media().unwrap().unwrap();
    let selected = media.lookup(&key, &BTreeMap::new()).unwrap();
    assert_eq!(reopened.media_audio(selected).unwrap(), audio);
    assert_eq!(reopened.media_timing(selected).unwrap().unwrap().active(200).count(), 1);
    let mut missing = bundle.clone();
    missing.required.insert(key.locale.clone(), false);
    missing.fallback.clear();
    assert!(
        Package::build(graph.clone(), false).unwrap().with_media(missing, files.clone()).is_err()
    );
    let mut stale = bundle.clone();
    stale.takes.get_mut(&key.token()).unwrap().source_revision = "wrong".into();
    assert!(
        Package::build(graph.clone(), false).unwrap().with_media(stale, files.clone()).is_err()
    );
    let mut untimed = bundle;
    untimed.takes.get_mut(&key.token()).unwrap().timing = None;
    assert!(Package::build(graph, false).unwrap().with_media(untimed, files).is_err());
}
