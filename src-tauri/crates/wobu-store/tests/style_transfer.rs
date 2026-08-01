use wobu_core::{AssetKind, AssetRole, LinkRole, NodeKind};
use wobu_store::{Error, Project, SaveOutcome, paths, transfer};

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0]);
    out
}

#[test]
fn style_replacement_preserves_destination_identity_and_copies_references() {
    let source_dir = tempfile::tempdir().unwrap();
    let mut source = Project::create(source_dir.path(), "Source").unwrap();
    let source_style = source
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| node.kind == NodeKind::StyleGuide)
        .unwrap();
    let asset = source.import_asset(&png(80, 60), AssetKind::Reference).unwrap().asset;
    source
        .link_asset(source_style.id, asset.id, AssetRole::Palette, Some(0.65))
        .unwrap();
    source
        .update_asset_link(source_style.id, asset.id, AssetRole::Palette, None, Some(false))
        .unwrap();
    source.set_cover_asset(source_style.id, Some(asset.id)).unwrap();
    let mut incoming = source.get_node(source_style.id).unwrap();
    incoming.notes_raw = "muted ink and a narrow amber palette".to_string();
    assert!(matches!(source.save_node(incoming).unwrap(), SaveOutcome::Saved(_)));
    let source_root = source.root().to_path_buf();
    drop(source);

    let preview = transfer::preview(&source_root).unwrap();
    assert_eq!(preview.default_root_id, Some(source_style.id));
    let choice = preview.candidates.iter().find(|item| item.root_id == source_style.id).unwrap();
    assert_eq!(choice.reference_count, 1);
    assert!(choice.replaces_singleton);
    assert!(preview.pinned_loras.is_empty());

    let destination_dir = tempfile::tempdir().unwrap();
    let mut destination = Project::create(destination_dir.path(), "Destination").unwrap();
    let existing_asset = destination
        .import_asset(&png(80, 60), AssetKind::Reference)
        .unwrap()
        .asset;
    assert_eq!(existing_asset.id, asset.id);
    let old = destination
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| node.kind == NodeKind::StyleGuide)
        .unwrap();
    let old_node = destination.get_node(old.id).unwrap();
    let canon = destination
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| node.kind == NodeKind::WorldBible)
        .unwrap();
    destination
        .add_node_link(canon.id, old.id, LinkRole::StyledBy, None, None)
        .unwrap();
    let outcome = destination
        .apply_transfer(transfer::stage(&source_root, source_style.id).unwrap())
        .unwrap();

    assert!(outcome.completed);
    assert_eq!(outcome.deduped_reference_count, 1);
    assert_eq!(outcome.imported_root_id, old.id);
    let imported = destination.get_node(old.id).unwrap();
    assert_eq!(imported.id, old.id);
    assert_eq!(imported.slug, old_node.slug);
    assert_eq!(imported.created_at, old_node.created_at);
    assert_eq!(imported.notes_raw, "muted ink and a narrow amber palette");
    assert_eq!(imported.cover_asset_id, Some(asset.id));
    assert_eq!(imported.asset_links[0].weight, 0.65);
    assert!(!imported.asset_links[0].enabled);
    assert!(destination.get_asset(asset.id).unwrap().is_some());
    assert_eq!(destination.list_assets().unwrap().len(), 1);
    assert_eq!(destination.get_node(canon.id).unwrap().links[0].to_id, old.id);
}

#[test]
fn subtree_ids_and_parents_remap_while_external_links_are_dropped() {
    let source_dir = tempfile::tempdir().unwrap();
    let mut source = Project::create(source_dir.path(), "Source").unwrap();
    let root = source.create_node(NodeKind::Setting, "Glass Coast", None).unwrap();
    let child = source.create_node(NodeKind::Setting, "Salt Port", Some(root.id)).unwrap();
    let outside = source.create_node(NodeKind::Setting, "Far Reach", None).unwrap();
    source
        .add_node_link(child.id, root.id, LinkRole::RelatedTo, None, None)
        .unwrap();
    source
        .add_node_link(child.id, outside.id, LinkRole::RelatedTo, None, None)
        .unwrap();
    let source_root = source.root().to_path_buf();
    drop(source);

    let destination_dir = tempfile::tempdir().unwrap();
    let mut destination = Project::create(destination_dir.path(), "Destination").unwrap();
    let outcome = destination
        .apply_transfer(transfer::stage(&source_root, root.id).unwrap())
        .unwrap();
    assert!(outcome.completed);
    assert_eq!(outcome.planned_node_count, 2);
    assert_eq!(outcome.dropped_external_link_count, 1);

    let imported_root = destination.get_node(outcome.imported_root_id).unwrap();
    let imported_child = destination
        .world_nodes()
        .unwrap()
        .iter()
        .find(|node| node.name == "Salt Port")
        .unwrap();
    assert_ne!(imported_root.id, root.id);
    assert_ne!(imported_child.id, child.id);
    assert_eq!(imported_child.parent_id, Some(imported_root.id));
    assert_eq!(imported_child.links.len(), 1);
    assert_eq!(imported_child.links[0].to_id, imported_root.id);
}

#[test]
fn a_missing_source_blob_blocks_staging_before_the_destination_changes() {
    let source_dir = tempfile::tempdir().unwrap();
    let mut source = Project::create(source_dir.path(), "Source").unwrap();
    let style = source
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| node.kind == NodeKind::StyleGuide)
        .unwrap();
    let asset = source.import_asset(&png(12, 12), AssetKind::Reference).unwrap().asset;
    source
        .link_asset(style.id, asset.id, AssetRole::Palette, None)
        .unwrap();
    let source_root = source.root().to_path_buf();
    let blob = paths::from_rel_string(&source_root, &asset.rel_path);
    drop(source);
    std::fs::remove_file(blob).unwrap();

    let preview = transfer::preview(&source_root).unwrap();
    let choice = preview.candidates.iter().find(|item| item.root_id == style.id).unwrap();
    assert_eq!(choice.missing_asset_count, 1);
    assert!(matches!(
        transfer::stage(&source_root, style.id),
        Err(Error::NoSuchAsset(_)) | Err(Error::Io { .. })
    ));
}

#[test]
fn a_project_cannot_import_from_itself_even_through_a_staged_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::create(dir.path(), "One World").unwrap();
    let style = project
        .list_nodes()
        .unwrap()
        .into_iter()
        .find(|node| node.kind == NodeKind::StyleGuide)
        .unwrap();
    let root = project.root().to_path_buf();
    drop(project);

    let bundle = transfer::stage(&root, style.id).unwrap();
    let mut same = Project::open(&root).unwrap();
    assert!(matches!(
        same.apply_transfer(bundle),
        Err(Error::TransferSameProject)
    ));
}
