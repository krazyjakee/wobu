//! The static wiki is a projection, never another project database. These
//! tests hold its public boundary: complete content and relative navigation on
//! success, and fail-closed destination rules before any output is claimed.

use std::fs;

use chrono::{DateTime, Utc};
use wobu_core::{
    AssetKind, AssetRole, Description, Generation, InfluenceSnapshot, Link, LinkRole, NodeKind,
    SectionValue,
};
use wobu_store::{Error, Project, SaveOutcome, wiki};

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0]);
    out
}

fn saved(outcome: SaveOutcome) -> wobu_core::Node {
    match outcome {
        SaveOutcome::Saved(node) => *node,
        SaveOutcome::Conflict { conflict_path } => panic!("unexpected conflict at {conflict_path}"),
    }
}

fn project() -> (tempfile::TempDir, Project) {
    let parent = tempfile::tempdir().unwrap();
    let project = Project::create(parent.path(), "Ash & <Ember>").unwrap();
    (parent, project)
}

#[test]
fn a_complete_export_has_safe_node_galleries_graph_and_relative_navigation() {
    let (_project_parent, mut project) = project();
    let guild = project.create_node(NodeKind::Culture, "Cinder Guild", None).unwrap();
    let reference = project.import_asset(&png(640, 480), AssetKind::Reference).unwrap().asset;
    let concept = project.import_asset(&png(768, 768), AssetKind::Generated).unwrap().asset;

    let mut character = project.create_node(NodeKind::Character, "Kael <script>", None).unwrap();
    character.summary = "A scout & mapmaker".into();
    character.notes_raw = "## Field notes\n<script>steal()</script>\n- carries chalk".into();
    character.tags = vec!["night & ash".into()];
    character.attributes.insert("home_region".into(), "Glass Coast".into());
    character.links = vec![Link::new(guild.id, LinkRole::MemberOf)];
    character.description = Some(Description::from_sections([(
        "silhouette".to_string(),
        SectionValue::Text("Tall, hooded, and <quiet>.".into()),
    )]));
    let character = saved(project.save_node(character).unwrap());
    saved(project.link_asset(character.id, reference.id, AssetRole::FullRef, Some(0.75)).unwrap());
    saved(project.set_cover_asset(character.id, Some(reference.id)).unwrap());

    project
        .record_generation(Generation {
            id: wobu_core::new_id(),
            node_id: character.id,
            created_at: "2026-08-01T12:00:00Z".parse::<DateTime<Utc>>().unwrap(),
            preset: "character_sheet".into(),
            view_type: None,
            user_prompt: "at dusk".into(),
            compiled_prompt: "full prompt".into(),
            negative_prompt: "text".into(),
            backend: "fixture".into(),
            model: "paper-crane".into(),
            seed: 42,
            params: Default::default(),
            output_asset_ids: vec![concept.id],
            influence_snapshot: InfluenceSnapshot { layers: vec![] },
        })
        .unwrap();

    let snapshot = project.wiki_snapshot().unwrap();
    let exports = tempfile::tempdir().unwrap();
    let destination = exports.path().join("ashfall-wiki");
    let result = wiki::export(snapshot, &destination).unwrap();

    assert_eq!(result.node_count, 4);
    assert_eq!(result.image_count, 2);
    assert_eq!(result.missing_images, 0);
    assert!(destination.join("index.html").is_file());
    assert!(destination.join("graph.html").is_file());
    assert!(destination.join("site.css").is_file());
    assert!(!destination.join(".wobu-export-incomplete").exists());
    assert!(destination.join(format!("media/originals/{}.png", reference.id)).is_file());
    assert!(destination.join(format!("media/originals/{}.png", concept.id)).is_file());

    let index = fs::read_to_string(destination.join("index.html")).unwrap();
    assert!(index.contains("Ash &amp; &lt;Ember&gt;"));
    assert!(index.contains("<h2>Characters</h2>"));
    assert!(index.contains(&format!("nodes/{}.html", character.id)));
    assert!(!index.contains(project.root().to_string_lossy().as_ref()));

    let page = fs::read_to_string(destination.join(format!("nodes/{}.html", character.id))).unwrap();
    assert!(page.contains("Kael &lt;script&gt;"));
    assert!(page.contains("&lt;script&gt;steal()&lt;/script&gt;"));
    assert!(!page.contains("<script>"));
    assert!(page.contains("Tall, hooded, and &lt;quiet&gt;."));
    assert!(page.contains("Full reference · 0.75"));
    assert!(page.contains("character_sheet · 2026-08-01 · paper-crane · seed 42"));
    assert!(page.contains(&format!("../media/originals/{}.png", concept.id)));
    assert!(page.contains(&format!("href=\"{}.html\"", guild.id)));

    let graph = fs::read_to_string(destination.join("graph.html")).unwrap();
    assert!(graph.contains("<svg class=\"world-graph\""));
    assert!(graph.contains("Member of"));
    assert!(graph.contains(&format!("nodes/{}.html", character.id)));
}

#[test]
fn a_missing_linked_blob_becomes_a_visible_placeholder_and_warning() {
    let (_project_parent, mut project) = project();
    let asset = project.import_asset(&png(320, 240), AssetKind::Reference).unwrap().asset;
    let node = project.create_node(NodeKind::Species, "Vashk", None).unwrap();
    saved(project.link_asset(node.id, asset.id, AssetRole::Palette, None).unwrap());
    let snapshot = project.wiki_snapshot().unwrap();
    fs::remove_file(project.root().join(&asset.rel_path)).unwrap();

    let exports = tempfile::tempdir().unwrap();
    let destination = exports.path().join("missing-image-wiki");
    let result = wiki::export(snapshot, &destination).unwrap();

    assert_eq!(result.missing_images, 1);
    let page = fs::read_to_string(destination.join(format!("nodes/{}.html", node.id))).unwrap();
    assert!(page.contains("Image unavailable"));
    assert!(page.contains("Palette · 1.00"));
}

#[test]
fn destinations_are_new_outside_paths_and_are_never_cleaned_up() {
    let (_project_parent, mut project) = project();
    let outside = tempfile::tempdir().unwrap();
    let existing = outside.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("mine.txt"), "keep me").unwrap();

    let error = wiki::export(project.wiki_snapshot().unwrap(), &existing).unwrap_err();
    assert!(matches!(error, Error::AlreadyExists(path) if path == existing));
    assert_eq!(fs::read_to_string(existing.join("mine.txt")).unwrap(), "keep me");

    let inside = project.root().join("published");
    let error = wiki::export(project.wiki_snapshot().unwrap(), &inside).unwrap_err();
    assert!(matches!(error, Error::ExportInsideProject(path) if path == inside));
    assert!(!inside.exists());
}

#[cfg(unix)]
#[test]
fn a_symlinked_parent_cannot_disguise_a_destination_inside_the_project() {
    use std::os::unix::fs::symlink;

    let (_project_parent, mut project) = project();
    let outside = tempfile::tempdir().unwrap();
    let disguised_parent = outside.path().join("elsewhere");
    symlink(project.root(), &disguised_parent).unwrap();
    let destination = disguised_parent.join("published");

    let error = wiki::export(project.wiki_snapshot().unwrap(), &destination).unwrap_err();
    assert!(matches!(error, Error::ExportInsideProject(path) if path == destination));
    assert!(!project.root().join("published").exists());
}

#[test]
fn malformed_generation_receipts_fail_before_the_destination_is_reserved() {
    let (_project_parent, mut project) = project();
    let snapshot = project.wiki_snapshot().unwrap();
    let receipt = project.root().join("generations/2026-08/broken.json");
    fs::create_dir_all(receipt.parent().unwrap()).unwrap();
    fs::write(&receipt, "{not json").unwrap();
    let exports = tempfile::tempdir().unwrap();
    let destination = exports.path().join("strict-wiki");

    let error = wiki::export(snapshot, &destination).unwrap_err();

    assert!(matches!(error, Error::MalformedGeneration { path, .. } if path == receipt));
    assert!(!destination.exists());
}
