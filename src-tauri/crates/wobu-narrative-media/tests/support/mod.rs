#![allow(dead_code)]
pub fn wav() -> Vec<u8> {
    let frames = 8000u32;
    let size = frames * 2;
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + size).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(8000u32.to_le_bytes());
    bytes.extend(16000u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(size.to_le_bytes());
    for i in 0..frames {
        let value = if i % 32 < 16 { 6000i16 } else { -6000 };
        bytes.extend(value.to_le_bytes());
    }
    bytes
}
pub fn row() -> wobu_narrative_media::Row {
    use std::collections::BTreeMap;
    use wobu_narrative_media::*;
    let locale = "en".parse().unwrap();
    let id = "00000000000000000000000001".to_string();
    Row {
        version: 1,
        key: Key { id: id.clone(), locale, form: wobu_narrative_locale::PluralCategory::Other },
        source: wobu_narrative_locale::SourceLine {
            id,
            slot: "00000000000000000000000002".into(),
            container: "00000000000000000000000003".into(),
            speaker: "Narrator".into(),
            text: "Hello.".into(),
            revision: "revision".into(),
            guard: "guard".into(),
            context: "Harbor".into(),
            delivery_notes: String::new(),
            placeholders: Default::default(),
            ready: true,
        },
        origin: "en".parse().unwrap(),
        translation_guard: None,
        text: "Hello.".into(),
        notes: Notes { pronunciation: String::new(), delivery: String::new() },
        parameters: BTreeMap::new(),
        media_guard: None,
        audio_path: "recordings/take.wav".into(),
        timing_path: None,
        audio_hash: None,
        timing_hash: None,
        ready: true,
    }
}
