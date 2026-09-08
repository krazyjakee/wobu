//! Creates a fresh, usable project including the original text, real speakers
//! and the linked quest. It never overwrites an existing project.
use std::{error::Error, path::Path};
use wobu_core::{Node, NodeKind};
use wobu_narrative::{Quest, StateDocument, TextAssetDocument, WorldDocument};
use wobu_store::{Project, TextFile};

pub fn create(parent: &Path) -> Result<Project, Box<dyn Error>> {
    let mut project = Project::create(parent, "Harbor Voices")?;
    for (id, name, voice) in [
        (
            "00000000000000000000000090",
            "Seawall keeper",
            "Practical, dry, short sentences. Concern appears through attention to harbour routines.",
        ),
        (
            "00000000000000000000000091",
            "Mara",
            "A watch clerk who separates direct observation from hearsay. Warm but precise.",
        ),
        (
            "00000000000000000000000092",
            "Iven",
            "A dockhand who questions the official account. Wry, conversational and concrete.",
        ),
    ] {
        let mut node = Node::new(NodeKind::Character, name)?;
        node.id = id.parse()?;
        node.attributes.insert("narrative_voice".into(), voice.into());
        project.save_node(node)?;
    }
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/narrative/harbor-voices");
    project.save_state(
        &StateDocument::parse(&std::fs::read_to_string(source.join("state.yaml"))?)?,
        None,
    )?;
    let mut world = WorldDocument::default();
    world.quests.push(Quest {
        id: "00000000000000000000000099".parse()?, name: "The missing light".into(),
        summary: "Investigate why the harbour watch lantern went dark. Distinguish the witnessed failure, reports and rumours; record only what the supplied scenario establishes.".into(),
        stages: vec!["investigating".parse()?], initial: "investigating".parse()?,
        transitions: vec![], scene_ids: vec![],
    });
    project.save_world(&world, None)?;
    for name in [
        "seawall-keeper-bark",
        "quay-talk-ambient",
        "mara-logbook-reaction",
        "harbour-watch-codex",
        "missing-light-quest-summary",
        "watch-nights-journal",
    ] {
        let asset = TextAssetDocument::parse(&std::fs::read_to_string(
            source.join(format!("texts/{name}.yaml")),
        )?)?
        .asset;
        let mut file = TextFile { asset, rel: wobu_store::narrative::text_rel(name), stamp: None };
        project.save_text_asset(&mut file)?;
    }
    Ok(project)
}

#[cfg(not(test))]
fn main() -> Result<(), Box<dyn Error>> {
    let parent = std::env::args_os().nth(1).ok_or("Pass a new parent directory")?;
    std::fs::create_dir_all(&parent)?;
    let project = create(Path::new(&parent))?;
    println!("{}", project.root().display());
    Ok(())
}
