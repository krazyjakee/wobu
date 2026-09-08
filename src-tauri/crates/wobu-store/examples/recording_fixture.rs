//! Offline recording handoff fixture: original prepared speech, never a provider call.
use std::{error::Error, path::Path};
use wobu_narrative::{
    DialogueSlot, GenerationPolicy, Name, SceneId, Speaker, Text, TextEntry, TextKind, Variant,
    review::{EditorialAction, PolicyScope},
};
use wobu_narrative_media::{
    interchange::{decode, encode},
    timing::{Cue, Kind, Track},
};
use wobu_store::{Project, project::narrative_review::ReviewRequest};
pub fn create(parent: &Path) -> Result<Project, Box<dyn Error>> {
    let mut p = Project::create(parent, "Recording harbor")?;
    let mut file =
        p.create_text_asset(TextKind::Codex, "Lantern keeper", Name::new("read_lantern").unwrap())?;
    let mut entry = TextEntry::new("The watch");
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants
        .push(Variant::new(Text::written("The harbor is quiet. Keep the lantern burning.")));
    entry.lines.push(slot);
    file.asset.entries.push(entry);
    p.save_text_asset(&mut file)?;
    for action in [
        EditorialAction::Approve,
        EditorialAction::Policy { scope: PolicyScope::Slot, policy: GenerationPolicy::Locked },
    ] {
        let view = p.review_scene(SceneId::from_raw(file.asset.id.raw()), None)?;
        let line = &view.lines[0];
        p.apply_review(&ReviewRequest {
            guard: view.guard.clone(),
            target: line.target.clone(),
            context_revision: line.context_revision.clone(),
            state_json: view.state_json.clone(),
            action,
        })?;
    }
    let locale = "en".parse()?;
    let (mut policy, guard) = p.media_policy()?;
    policy.required.insert(locale, false);
    policy.timing.insert("en".parse()?);
    p.save_media_policy(policy, &guard)?;
    let mut rows = decode(&p.media_export(&"en".parse()?, false)?, false)?;
    let audio = include_bytes!("../../../../examples/recording-handoff/harbor.wav");
    let info = wobu_narrative_media::wav::inspect(audio)?;
    let hash = blake3::hash(audio).to_hex().to_string();
    let row = &mut rows[0];
    row.audio_hash = Some(hash.clone());
    row.timing_path = Some(row.audio_path.replace(".wav", ".timing.json"));
    let path = parent.join(&row.audio_path);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, audio)?;
    // Hand-authored cue intervals demonstrate the interchange; they are not forced alignment.
    let track = Track {
        version: 1,
        audio_hash: hash,
        duration_ms: info.duration_ms,
        cues: vec![
            Cue {
                start_ms: 0,
                end_ms: 1200,
                kind: Kind::Word,
                value: "The harbor is quiet.".into(),
            },
            Cue {
                start_ms: 1200,
                end_ms: info.duration_ms,
                kind: Kind::Word,
                value: "Keep the lantern burning.".into(),
            },
            Cue { start_ms: 100, end_ms: 900, kind: Kind::Viseme, value: "aa".into() },
            Cue { start_ms: 1300, end_ms: 2200, kind: Kind::Viseme, value: "ih".into() },
        ],
    };
    let timing = serde_json::to_vec_pretty(&track)?;
    row.timing_hash = Some(blake3::hash(&timing).to_hex().to_string());
    std::fs::write(parent.join(row.timing_path.as_ref().unwrap()), timing)?;
    std::fs::write(parent.join("recording.csv"), encode(&rows, true)?)?;
    std::fs::write(parent.join("recording.json"), encode(&rows, false)?)?;
    let mut unknown = rows[0].clone();
    unknown.key.id = wobu_core::new_id().to_string();
    rows.push(unknown);
    std::fs::write(parent.join("partial.json"), encode(&rows, false)?)?;
    println!("{}", p.root().display());
    Ok(p)
}
fn main() -> Result<(), Box<dyn Error>> {
    let directory = std::env::args().nth(1).ok_or("Pass an existing empty parent directory.")?;
    create(Path::new(&directory))?;
    Ok(())
}
