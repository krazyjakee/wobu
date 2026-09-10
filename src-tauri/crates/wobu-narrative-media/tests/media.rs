mod support;
use std::collections::BTreeMap;
use wobu_narrative_media::{self as media, *};
#[test]
fn wav_checks_actual_format_lengths_alignment_and_bounded_duration() {
    let original = support::wav();
    let info = wav::inspect(&original).unwrap();
    assert_eq!(info.duration_ms, 1000);
    assert_eq!(info.frames, 8000);
    for offset in [4, 16, 20, 22, 24, 28, 32, 34, 40] {
        let mut broken = original.clone();
        broken[offset] = 255;
        assert!(wav::inspect(&broken).is_err(), "offset {offset}");
    }
    assert!(wav::inspect(&original[..original.len() - 1]).is_err());
    assert!(wav::inspect(b"not wave audio").is_err());
}
#[test]
fn timing_is_hash_bound_ordered_per_kind_and_native_lookup_is_half_open() {
    let hash = blake3::hash(&support::wav()).to_hex().to_string();
    let mut track = timing::Track {
        version: 1,
        audio_hash: hash.clone(),
        duration_ms: 1000,
        cues: vec![
            timing::Cue {
                start_ms: 0,
                end_ms: 500,
                kind: timing::Kind::Word,
                value: "Hello".into(),
            },
            timing::Cue {
                start_ms: 0,
                end_ms: 400,
                kind: timing::Kind::Viseme,
                value: "aa".into(),
            },
        ],
    };
    track.validate(&hash, 1000).unwrap();
    assert_eq!(track.active(100).count(), 2);
    assert_eq!(track.active(500).count(), 0);
    assert!(track.validate("wrong", 1000).is_err());
    track.cues[0].end_ms = 1001;
    assert!(track.validate(&hash, 1000).is_err());
    track.cues[0].end_ms = 500;
    track.cues.push(timing::Cue {
        start_ms: 400,
        end_ms: 600,
        kind: timing::Kind::Word,
        value: "overlap".into(),
    });
    assert!(track.validate(&hash, 1000).is_err());
    track.cues.pop();
    track.cues[1].value = "engine_specific_999".into();
    assert!(track.validate(&hash, 1000).is_err());
}
#[test]
fn csv_and_json_roundtrip_quotes_newlines_rtl_and_exact_revision_metadata() {
    let mut row = support::row();
    row.text = "مرحباً، \"{name}\"!\nThe harbor.".into();
    row.parameters.insert("name".into(), "ليلى".into());
    row.notes.pronunciation = "ليلى / Lay-la".into();
    row.notes.delivery = "Quiet,\nthen emphatic".into();
    for csv in [true, false] {
        let encoded = interchange::encode(std::slice::from_ref(&row), csv).unwrap();
        assert_eq!(interchange::decode(&encoded, csv).unwrap(), vec![row.clone()]);
        assert_eq!(encoded, interchange::encode(std::slice::from_ref(&row), csv).unwrap());
    }
    assert_eq!(row.spoken_text().unwrap(), "مرحباً، \"ليلى\"!\nThe harbor.");
    row.parameters.clear();
    assert!(row.spoken_text().is_err());
}
#[test]
fn preview_never_rebinds_unknown_duplicate_stale_locale_or_concurrent_take() {
    let row = support::row();
    let current = BTreeMap::from([(row.key.token(), row.clone())]);
    assert!(preview(std::slice::from_ref(&row), &current).is_empty());
    let mut changed = row.clone();
    changed.source.revision = "new".into();
    assert!(preview(&[changed], &current).iter().any(|d| d.code == "stale_script"));
    let mut foreign = row.clone();
    foreign.key.locale = "ar".parse().unwrap();
    assert!(preview(&[foreign], &current).iter().any(|d| d.code == "unknown"));
    assert!(preview(&[row.clone(), row.clone()], &current).iter().any(|d| d.code == "duplicate"));
    let mut conflicting = row;
    conflicting.media_guard = Some("winner".into());
    assert!(preview(&[conflicting], &current).iter().any(|d| d.code == "media_conflict"));
}
#[test]
fn portable_blob_identity_refuses_mismatched_hash_or_paths() {
    let hash = blake3::hash(&support::wav()).to_hex().to_string();
    let mut blob = media::Blob { path: format!("assets/media/{hash}.wav"), hash, bytes: 42 };
    validate_blob(&blob, "wav").unwrap();
    blob.path = "../outside.wav".into();
    assert!(validate_blob(&blob, "wav").is_err());
}
