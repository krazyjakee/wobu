use std::collections::BTreeMap;
use wobu_narrative::{BeatId, DialogueSlotId, SceneId, Speaker, VariantId};
use wobu_narrative_context::{FrozenContext, Options, Selection, content_hash};
use wobu_narrative_generation::*;

fn fixture() -> (FrozenRequest, serde_json::Value) {
    let target = Selection {
        scene: SceneId::new(),
        beat: BeatId::new(),
        slot: DialogueSlotId::new(),
        variant: None,
    };
    let mut context = FrozenContext {
        version: 1,
        options: Options { selection: target.clone(), state: BTreeMap::new(), token_budget: 4000 },
        fragments: vec![],
        omitted: vec![],
        diagnostics: vec![],
        dependencies: BTreeMap::new(),
        queries: vec![],
        request: "The harbour keeper has only heard a rumour.".into(),
        estimated_tokens: 15,
        ready: true,
        hash: String::new(),
    };
    context.hash = content_hash(&context);
    let candidate = Candidate {
        slot_id: target.slot,
        variant_id: VariantId::new(),
        speaker: Speaker::Narrator,
        text: String::new(),
    };
    let request = FrozenRequest {
        version: REQUEST_VERSION,
        source_schema_version: wobu_narrative::SOURCE_SCHEMA_VERSION,
        request_id: wobu_core::new_id(),
        batch_id: wobu_core::new_id(),
        target,
        candidate_variant_id: candidate.variant_id,
        speaker: Speaker::Narrator,
        analysis: None,
        expected_scene_hash: "scene".into(),
        compiled_graph_hash: "graph".into(),
        expected_text_revision: None,
        expected_policy: None,
        provider: "test".into(),
        model: "fixture".into(),
        settings: Settings { max_output_tokens: 512 },
        expected_slot_policy: wobu_narrative::GenerationPolicy::Edited,
        prompt_version: PROMPT_VERSION,
        output_schema_version: OUTPUT_SCHEMA_VERSION,
        output_schema: output_schema(),
        system: SYSTEM.into(),
        prompt: prompt(&context, &candidate),
        context,
    };
    let output = serde_json::to_value(Output {
        lines: vec![Candidate { text: "They say the beacon went dark.".into(), ..candidate }],
    })
    .unwrap();
    (request, output)
}

#[test]
fn exact_authorised_prose_roundtrips_without_executable_fields() {
    let (request, output) = fixture();
    let candidate = request.validate_output(&output.to_string()).unwrap();
    assert_eq!(candidate.text, "They say the beacon went dark.");
    let request: FrozenRequest =
        serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
    request.validate().unwrap();
    assert_eq!(request.hash(), request.clone().hash());
}

#[test]
fn rejects_missing_duplicate_unknown_and_logic_injection() {
    let (request, output) = fixture();
    for bad in [
        serde_json::json!({"lines":[]}),
        serde_json::json!({"lines":[output["lines"][0],output["lines"][0]]}),
        serde_json::json!({"lines":output["lines"],"effects":[]}),
    ] {
        assert!(request.validate_output(&bad.to_string()).is_err());
    }
    for field in ["effects", "conditions", "next", "choices", "commands"] {
        let mut bad = output.clone();
        bad["lines"][0][field] = serde_json::json!([]);
        assert!(request.validate_output(&bad.to_string()).is_err(), "{field}");
    }
    let duplicate = format!("{{\"lines\":{},\"lines\":{}}}", output["lines"], output["lines"]);
    assert!(request.validate_output(&duplicate).is_err());
}

#[test]
fn rejects_changed_identities_bad_lengths_and_incomplete_json() {
    let (request, output) = fixture();
    for (field, value) in [
        ("slot_id", serde_json::json!(DialogueSlotId::new())),
        ("variant_id", serde_json::json!(VariantId::new())),
        ("speaker", serde_json::json!("player")),
        ("text", serde_json::json!(" \n")),
        ("text", serde_json::json!("x".repeat(MAX_TEXT_CHARS + 1))),
        ("text", serde_json::json!("bad\u{0001}control")),
    ] {
        let mut bad = output.clone();
        bad["lines"][0][field] = value;
        assert!(request.validate_output(&bad.to_string()).is_err(), "{field}");
    }
    assert!(request.validate_output("{\"lines\":[").is_err());
    assert!(request.validate_output(&" ".repeat(MAX_RESPONSE_BYTES + 1)).is_err());
}

#[test]
fn frozen_contract_rejects_future_versions_tampered_context_and_lock() {
    let (request, _) = fixture();
    for version in [1, 2] {
        let mut supported = request.clone();
        supported.version = version;
        supported.validate().unwrap();
        let bytes = serde_json::to_vec(&supported).unwrap();
        let restored: FrozenRequest = serde_json::from_slice(&bytes).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.hash(), supported.hash());
        assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
    }
    for version in [0, REQUEST_VERSION + 1] {
        let mut bad = request.clone();
        bad.version = version;
        assert!(bad.validate().is_err());
    }
    let mut bad = request.clone();
    bad.context.request.push_str(" changed");
    assert!(bad.validate().is_err());
    let mut bad = request.clone();
    bad.expected_policy = Some(wobu_narrative::GenerationPolicy::Locked);
    assert!(bad.validate().is_err());
    let mut bad = request.clone();
    bad.prompt.push_str(" override");
    assert!(bad.validate().is_err());
    let mut bad = request;
    bad.target.variant = Some(VariantId::new());
    assert!(bad.validate().is_err());
}

#[test]
fn source_capabilities_preserve_frozen_v1_and_reject_future_versions() {
    let (mut request, _) = fixture();
    request.version = 1;
    request.source_schema_version = 1;
    let bytes = serde_json::to_vec(&request).unwrap();
    let hash = request.hash();
    request.validate().unwrap();
    let decoded: FrozenRequest = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded.hash(), hash);
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
    request.source_schema_version = 2;
    request.validate().unwrap();
    request.source_schema_version = 3;
    assert!(request.validate().is_err());
}
