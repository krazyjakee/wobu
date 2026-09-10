//! cargo run -p wobu-store --example variant_matrix_fixture -- /tmp/wobu-170-native-fixture
//! Creates canonical source and explicit policy only; no provider or credentials.
use wobu_narrative::{
    Destination, DialogueSlot, GenerationPolicy, Outcome, Speaker, StateDocument,
};
use wobu_store::{Project, SceneFile};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let parent = std::env::args().nth(1).ok_or("Supply an existing parent directory")?;
    let f = wobu_narrative_variants::example::fixture();
    let mut project =
        Project::create(std::path::Path::new(&parent), "Seven hearing configurations")?;
    project.save_state(&StateDocument::new(f.schema.iter().cloned().collect()), None)?;
    project.save_world(&f.world, None)?;
    let mut scene = f.scenes[0].clone();
    scene.beats[0]
        .outcomes
        .push(Outcome::new(Destination::End { label: "Hearing complete".into() }));
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.policy = GenerationPolicy::Generated;
    scene.beats[0].dialogue.push(slot);
    let mut file = SceneFile { scene, rel: "narrative/scenes/hearing.yaml".into(), stamp: None };
    project.save_scene(&mut file)?;
    let capture = project.narrative_analysis_capture()?;
    project.save_narrative_analysis_policy(f.policy, &capture)?;
    println!("{}", project.root().display());
    println!(
        "Open Hearing → Variants → Save policy and plan. Expect 12 potential, 7 included, 5 excluded, 0 unknown. Select Target line 2 for the empty Generated slot. No provider calls are made by this fixture."
    );
    Ok(())
}
