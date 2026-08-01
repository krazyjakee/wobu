//! The schema `wobu-core` sends and the validator `wobu-llm` applies have to
//! describe the same thing for every kind. Nothing else in the workspace checks
//! that pair, and a divergence would look like a provider "ignoring" a schema it
//! actually honoured.

use serde_json::{Map, Value, json};
use wobu_core::kind::{NodeKind, kind_registry};
use wobu_core::schema::description_schema;
use wobu_llm::{Error, validate_description};

/// A response built from the schema alone — no peeking at the registry — so a
/// schema that drifts from what the validator expects fails here.
fn response_from_schema(kind: NodeKind) -> Value {
    let schema = description_schema(kind);
    let mut object = Map::new();
    for (key, property) in schema["properties"].as_object().unwrap() {
        let value = match property["type"].as_str().unwrap() {
            "string" => json!("Ash-glazed ceramic plate over oiled leather."),
            "array" if property["items"]["pattern"].is_string() => json!(["#2b2118", "#c2703a"]),
            "array" => json!(["Ember-lit throat vents"]),
            other => panic!("{kind} declares an unhandled property type {other}"),
        };
        object.insert(key.clone(), value);
    }
    Value::Object(object)
}

#[test]
fn every_kind_can_round_trip_a_response_built_from_its_own_schema() {
    for def in kind_registry() {
        let validated = validate_description(def.kind, &response_from_schema(def.kind))
            .unwrap_or_else(|e| panic!("{} rejected its own schema's shape: {e}", def.kind));

        let keys: Vec<&str> = validated.description.sections.keys().map(String::as_str).collect();
        let declared: Vec<&str> = def.sections.iter().map(|s| s.key).collect();
        assert_eq!(keys, declared, "{} lost or reordered a section", def.kind);
        assert!(validated.extra_sections.is_empty(), "{}", def.kind);
        assert!(!validated.description.is_empty(), "{}", def.kind);
    }
}

#[test]
fn dropping_any_required_section_fails_for_every_kind() {
    // Guards the case where a kind gains a section: the schema requires it, so
    // the validator must insist on it too, without anyone wiring it up by hand.
    for def in kind_registry() {
        let required = description_schema(def.kind)["required"].clone();
        for key in required.as_array().unwrap() {
            let key = key.as_str().unwrap();
            let mut response = response_from_schema(def.kind);
            response.as_object_mut().unwrap().remove(key);
            let err = validate_description(def.kind, &response)
                .expect_err(&format!("{} accepted a response missing {key}", def.kind));
            assert!(matches!(err, Error::MissingSection { section } if section == key));
            assert!(err.is_retryable(), "a missing section must never reach a node");
        }
    }
}

#[test]
fn every_kind_rejects_a_colour_name_in_its_palette() {
    // Every kind declares `palette`, and a non-hex entry is the failure mode a
    // provider is most likely to produce, since prose is what it does all day.
    for def in kind_registry() {
        let mut response = response_from_schema(def.kind);
        assert!(response.get("palette").is_some(), "{} has no palette", def.kind);
        response["palette"] = json!(["burnt ochre"]);
        assert!(matches!(
            validate_description(def.kind, &response),
            Err(Error::NotAHexColor { section: "palette", .. })
        ));
    }
}
