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
