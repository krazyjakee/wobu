//! What the Text library command layer promises, against a real project folder.
//!
//! Drives the free functions rather than the `#[tauri::command]` wrappers, for
//! the reason `commands/narrative/tests.rs` gives: the wrapper is one call to
//! `AppState::with`, and standing up a managed-state harness would test Tauri.

use wobu_narrative::{DialogueSlot, GenerationPolicy, Speaker, Text, TextEntry, Variant};
use wobu_store::Project;

use super::*;
// The same self-removing project folder the narrative command tests use. A
// second copy would be a second place a temp directory can be leaked.
use crate::commands::narrative::tests::{Temp, project};

fn stamp_of(view: &TextFileView) -> Precondition {
    Precondition::Stamp { stamp: view.stamp.clone().expect("a saved asset has a stamp") }
}

fn spoken(body: &str) -> DialogueSlot {
    let mut slot = DialogueSlot::new(Speaker::Narrator);
    slot.variants.push(Variant::new(Text::written(body)));
    slot
}

fn created(project: &mut Project, kind: TextKind, name: &str, event: &str) -> TextFile {
    project.create_text_asset(kind, name, Name::new(event).unwrap()).unwrap()
}

#[test]
fn every_kind_can_be_created_listed_and_reopened() {
    let temp = Temp::new();
    let mut project = project(&temp);
    for (index, kind) in TextKind::ALL.into_iter().enumerate() {
        created(&mut project, kind, &format!("Asset {index}"), &format!("event_{index}"));
    }

    let catalog = project.text_catalog().unwrap();
    assert_eq!(catalog.assets.len(), 6);
    assert!(catalog.unreadable.is_empty());

    for summary in &catalog.assets {
        let reopened = project.load_text_asset(summary.id).unwrap();
        assert_eq!(reopened.asset.kind, summary.kind);
        // The default repeat policy comes from the template, not from a form
        // the caller filled in.
        assert_eq!(reopened.asset.repeat, summary.kind.default_repeat());
    }
}

#[test]
fn a_new_asset_lands_on_its_own_slug_and_a_repeat_name_does_not_overwrite_it() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let first = created(&mut project, TextKind::Bark, "Gate guard", "player_passes_gate");
    let second = created(&mut project, TextKind::Bark, "Gate guard", "player_passes_gate");

    assert_eq!(first.rel, "narrative/texts/gate-guard.yaml");
    assert_ne!(second.rel, first.rel);
    assert_eq!(project.text_catalog().unwrap().assets.len(), 2);
}

#[test]
fn a_save_needs_the_stamp_it_read_and_a_stale_one_is_parked_as_a_conflict() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let file = created(&mut project, TextKind::Bark, "Gate guard", "player_passes_gate");
    let view = TextFileView::of(&file);

    let mut asset = view.asset.clone();
    asset.entries.push(TextEntry { lines: vec![spoken("Move along.")], ..TextEntry::new("One") });
    let saved = save_text(&mut project, asset.clone(), None, &stamp_of(&view)).unwrap();
    assert_eq!(saved.asset.entries.len(), 1);

    // The first save's stamp is now stale. Presenting it again must not
    // overwrite the newer file.
    let mut later = saved.asset.clone();
    later.entries.push(TextEntry { lines: vec![spoken("Papers.")], ..TextEntry::new("Two") });
    let error = save_text(&mut project, later, None, &stamp_of(&view)).unwrap_err();
    assert_eq!(error.code, crate::error::Code::Conflict);
    assert_eq!(project.load_text_asset(asset.id).unwrap().asset.entries.len(), 1);
}

#[test]
fn undo_cannot_borrow_the_current_stamp_to_write_over_a_newer_file() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let file = created(&mut project, TextKind::Codex, "Beacons", "codex_opened");
    let error = save_text(&mut project, file.asset, None, &Precondition::Current).unwrap_err();
    assert_eq!(error.code, crate::error::Code::Invalid);
}

#[test]
fn an_existing_asset_is_written_where_the_catalog_says_and_not_where_a_caller_asks() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let file = created(&mut project, TextKind::Journal, "First night", "day_ends");
    let view = TextFileView::of(&file);

    let saved =
        save_text(&mut project, view.asset.clone(), Some("somewhere-else"), &stamp_of(&view))
            .unwrap();
    assert_eq!(saved.rel, view.rel);
    assert_eq!(project.text_catalog().unwrap().assets.len(), 1);
}

#[test]
fn diagnostics_read_the_unsaved_document_when_one_is_supplied() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let file = created(&mut project, TextKind::Codex, "Beacons", "codex_opened");

    // Saved: no entries yet, which is the one thing wrong with it.
    let schema = project.state_schema().unwrap();
    let saved: Vec<_> = file.asset.diagnostics(&schema).iter().map(DiagnosticView::of).collect();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].code, "no_text_entries");
    assert_eq!(saved[0].kind, "textAsset");

    // The editor's unsaved copy has one, and the list follows it.
    let mut edited = file.asset.clone();
    edited
        .entries
        .push(TextEntry { lines: vec![spoken("Older than the harbour.")], ..TextEntry::new("A") });
    assert!(edited.diagnostics(&schema).is_empty());
}

#[test]
fn a_broken_speaker_is_reported_against_the_line_that_holds_it() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let mut file = created(&mut project, TextKind::Codex, "Beacons", "codex_opened");
    let mut line = spoken("Older than the harbour.");
    line.speaker = Speaker::Entity(wobu_core::new_id());
    file.asset.entries.push(TextEntry { lines: vec![line], ..TextEntry::new("A") });

    let schema = project.state_schema().unwrap();
    let views: Vec<_> = file.asset.diagnostics(&schema).iter().map(DiagnosticView::of).collect();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].code, "voice_not_allowed");
    assert_eq!(views[0].kind, "textLine");
    assert!(views[0].entry_id.is_some());
    assert!(views[0].slot_id.is_some());
}

#[test]
fn deleting_an_asset_removes_it_from_the_catalog_and_refuses_a_second_time() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let file = created(&mut project, TextKind::Reaction, "Relic", "relic_taken");
    project.delete_text_asset(file.asset.id).unwrap();
    assert!(project.text_catalog().unwrap().assets.is_empty());
    assert!(project.delete_text_asset(file.asset.id).is_err());
}

#[test]
fn supporting_text_moves_the_source_fingerprint_a_build_is_keyed_on() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let before = project.narrative_fingerprint().unwrap();
    let file = created(&mut project, TextKind::Bark, "Gate guard", "player_passes_gate");
    let after = project.narrative_fingerprint().unwrap();
    assert_ne!(before, after, "a new bark is new content and must change the build identity");

    let view = TextFileView::of(&file);
    let mut asset = view.asset.clone();
    asset.entries.push(TextEntry { lines: vec![spoken("Move along.")], ..TextEntry::new("One") });
    save_text(&mut project, asset, None, &stamp_of(&view)).unwrap();
    assert_ne!(project.narrative_fingerprint().unwrap(), after);
}

#[test]
fn a_locked_asset_reports_itself_as_closed_to_generation_through_the_saved_document() {
    let temp = Temp::new();
    let mut project = project(&temp);
    let file = created(&mut project, TextKind::Bark, "Gate guard", "player_passes_gate");
    let view = TextFileView::of(&file);

    let mut asset = view.asset.clone();
    asset.policy = GenerationPolicy::Locked;
    asset.entries.push(TextEntry { lines: vec![spoken("Move along.")], ..TextEntry::new("One") });
    save_text(&mut project, asset.clone(), None, &stamp_of(&view)).unwrap();

    // The lock survives the round trip through YAML, which is what makes it
    // hold for a caller that never went near the UI.
    assert!(!project.load_text_asset(asset.id).unwrap().asset.may_generate());
}
