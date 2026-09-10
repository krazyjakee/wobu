//! Reproducible on-disk scale input for #194, not an authoring or approval fixture.
//! Run with a new parent directory; an existing project is never overwritten.
use std::{error::Error, fs, path::PathBuf, time::Instant};
use wobu_core::Id;
use wobu_narrative::{
    Beat, BeatId, Destination, DialogueSlot, DialogueSlotId, Outcome, OutcomeId, Quest, Scene,
    SceneDocument, SceneId, Speaker, Text, Variant, VariantId, WorldDocument,
};
use wobu_store::Project;

const SCENES: usize = 1_000;
const SLOTS: usize = 50;

fn id(value: usize) -> Id {
    format!("{value:026}").parse().expect("decimal digits form a valid fixed-width ULID")
}

fn main() -> Result<(), Box<dyn Error>> {
    let parent = PathBuf::from(std::env::args_os().nth(1).ok_or("Pass a new parent directory")?);
    fs::create_dir_all(&parent)?;
    let project = Project::create(&parent, "Narrative scale")?;
    let root = project.root().to_owned();
    let index = project.index_path();
    drop(project);
    let source_dir = root.join("narrative/scenes");
    fs::create_dir_all(&source_dir)?;
    let started = Instant::now();
    let mut source_bytes = 0;
    let mut scene_ids = Vec::new();
    for number in 0..SCENES {
        let mut scene = Scene::new(format!("Ashfall hearing {number:04}"));
        scene.id = SceneId::from_raw(id(number + 1));
        scene.summary = format!("District {} investigates the missing harbor logbook.", number % 8);
        let mut beat = Beat::new("Compare the witnesses");
        beat.id = BeatId::from_raw(id(100_000 + number));
        for line in 0..SLOTS {
            let serial = number * SLOTS + line;
            let mut slot =
                DialogueSlot::new(if line % 2 == 0 { Speaker::Narrator } else { Speaker::Player });
            slot.id = DialogueSlotId::from_raw(id(1_000_000 + serial));
            let mut variant = Variant::new(Text::written(format!(
                "Witness {line:02} recalls the harbor bell. ASHFALL-PHRASE-{number:04} records district {} evidence.",
                number % 8
            )));
            variant.id = VariantId::from_raw(id(2_000_000 + serial));
            slot.variants.push(variant);
            beat.dialogue.push(slot);
        }
        let mut end = Outcome::new(Destination::End { label: "Evidence recorded".into() });
        end.id = OutcomeId::from_raw(id(3_000_000 + number));
        beat.outcomes.push(end);
        scene.beats.push(beat);
        scene_ids.push(scene.id);
        let text = SceneDocument::new(scene).to_yaml()?;
        source_bytes += text.len();
        fs::write(source_dir.join(format!("{number:04}.yaml")), text)?;
    }
    let mut world = WorldDocument::default();
    for number in 0..20 {
        world.quests.push(Quest {
            id: id(4_000_000 + number),
            name: format!("Harbor inquiry {number:02}"),
            summary: "Overlapping district investigations".into(),
            stages: vec![wobu_narrative::Name::new("investigating")?.into()],
            initial: "investigating".parse()?,
            transitions: vec![],
            scene_ids: scene_ids
                .iter()
                .enumerate()
                .filter(|(scene, _)| scene % 20 == number || (scene + 1) % 20 == number)
                .map(|(_, id)| *id)
                .collect(),
        });
    }
    fs::write(root.join("narrative/world.yaml"), world.to_yaml()?)?;
    let fixture_ms = started.elapsed().as_secs_f64() * 1_000.0;
    // This index belongs to the project created above. Source remains intact.
    fs::remove_file(&index)?;
    let started = Instant::now();
    let project = Project::open(&root)?;
    let cold_open_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let started = Instant::now();
    let catalog = project.scene_catalog()?;
    let catalog_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let selected = scene_ids[731];
    let mut selected_get_ms = Vec::new();
    for _ in 0..8 {
        let started = Instant::now();
        let file = project.load_scene(selected)?;
        assert_eq!(file.scene.beats[0].dialogue.len(), SLOTS);
        selected_get_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "project": root,
            "scenes": catalog.ids().len(),
            "slots": SCENES * SLOTS,
            "scene_source_bytes": source_bytes,
            "fixture_ms": fixture_ms,
            "cold_open_ms": cold_open_ms,
            "catalog_ms": catalog_ms,
            "selected_scene": selected,
            "selected_get_ms": selected_get_ms,
            "scope": "Real filesystem/store baseline; no IPC, webview or approval claim"
        }))?
    );
    Ok(())
}
