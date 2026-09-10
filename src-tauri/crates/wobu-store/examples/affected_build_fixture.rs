//! Reproducible native #169 walkthrough data. No provider calls or credentials.
//! cargo run -p wobu-store --example affected_build_fixture -- /tmp/build-demo [483]
use wobu_narrative::{
    Beat, Destination, DialogueSlot, GenerationPolicy, Outcome, Participant, Speaker, Text, Variant,
};
use wobu_store::Project;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let directory = args.next().ok_or("Supply an existing parent directory")?;
    let count = args.next().map(|s| s.parse::<usize>()).transpose()?.unwrap_or(3);
    if !(3..=10_000).contains(&count) {
        return Err("Choose 3–10000 affected lines".into());
    }
    let mut project =
        Project::create(std::path::Path::new(&directory), "Affected build walkthrough")?;
    let mut actor = project.create_node(wobu_core::NodeKind::Character, "Harbour keeper", None)?;
    actor.attributes.insert("narrative_voice".into(), serde_json::json!("Patient and reassuring."));
    project.save_node(actor.clone())?;
    let mut spoken = project.create_scene("The keeper's warning")?;
    spoken.scene.summary = "The keeper warns travellers about the approaching storm.".into();
    spoken.scene.participants.push(Participant { entity: actor.id, role: "Keeper".into() });
    let mut beat = Beat::new("Watch the storm");
    beat.outcomes.push(Outcome::new(Destination::End { label: "Warning heard".into() }));
    for i in 0..count {
        let policy = if i == 0 {
            GenerationPolicy::Generated
        } else if i % 2 == 1 {
            GenerationPolicy::Edited
        } else {
            GenerationPolicy::Locked
        };
        let mut line = DialogueSlot::new(Speaker::Entity(actor.id));
        line.policy = policy;
        let mut variant = Variant::new(Text::written(format!(
            "Keep to the marked harbour path. Warning {}.",
            i + 1
        )));
        variant.text.lifecycle.policy =
            if policy == GenerationPolicy::Generated { GenerationPolicy::Edited } else { policy };
        line.variants.push(variant);
        beat.dialogue.push(line);
    }
    spoken.scene.beats.push(beat);
    project.save_scene(&mut spoken)?;
    let view = project.review_scene(spoken.scene.id, None)?;
    let line = &view.lines[0];
    project.apply_review(&wobu_store::project::narrative_review::ReviewRequest {
        guard: view.guard.clone(),
        target: line.target.clone(),
        context_revision: line.context_revision.clone(),
        state_json: view.state_json.clone(),
        action: wobu_narrative::review::EditorialAction::Policy {
            scope: wobu_narrative::review::PolicyScope::Variant,
            policy: GenerationPolicy::Generated,
        },
    })?;
    let mut independent = project.create_scene("Unrelated arrival sign")?;
    let mut beat = Beat::new("Read the sign");
    beat.outcomes.push(Outcome::new(Destination::End { label: "Sign read".into() }));
    let mut line = DialogueSlot::new(Speaker::Narrator);
    line.variants.push(Variant::new(Text::written("Welcome to the harbour.")));
    beat.dialogue.push(line);
    independent.scene.beats.push(beat);
    project.save_scene(&mut independent)?;
    // Local saves already recorded the old context. This edit should include
    // only the keeper's lines in Build → Affected content.
    actor
        .attributes
        .insert("narrative_voice".into(), serde_json::json!("Urgent, clipped warnings."));
    project.save_node(actor)?;
    let affected = project.narrative_affected()?;
    if affected.len() != count {
        return Err(format!("Expected {count} affected lines; found {}", affected.len()).into());
    }
    println!("{}", project.root().display());
    println!(
        "{count} affected lines; 1 unrelated line. Open Build → Affected content → Plan work."
    );
    Ok(())
}
