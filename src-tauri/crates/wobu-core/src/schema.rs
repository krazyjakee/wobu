//! The JSON Schema handed to a provider when asking for a description.
//!
//! Derived from the kind registry rather than written per provider or per kind,
//! so adding a section to a kind in `kind.rs` changes the schema everywhere at
//! once. A hand-written schema would drift the moment someone edited the
//! registry, and the symptom would be a model that keeps filling in a section
//! the app then discards.
//!
//! The shape is deliberately flat: one object, one property per section, no
//! nested objects. Google states that deeply nested schemas may be rejected and
//! that only a subset of JSON Schema is supported (`docs/08-providers.md`), and
//! Anthropic tool-use is easier to steer flat. The same schema has to satisfy
//! both, so it stays at the intersection of what they document.

use serde_json::{Map, Value, json};

use crate::kind::{NodeKind, SectionDef, SectionValueKind, kind_def};

/// The one section whose items have a fixed form. Named here because both the
/// schema and the `wobu-llm` validator have to agree on which list gets the
/// colour treatment.
pub const PALETTE_KEY: &str = "palette";

/// The `pattern` we put on palette entries: six hex digits with a leading `#`.
///
/// Shorthand (`#fff`) and alpha (`#rrggbbaa`) are deliberately rejected. Palette
/// entries are read back as swatches and compiled into prompt text, so allowing
/// three forms of the same colour would mean every consumer normalising first,
/// and alpha has no meaning at all for a colour-conditioning pass.
///
/// Kept in sync with [`is_hex_color`] by hand — `wobu-core` has no regex engine,
/// and this string is only a hint to the provider. [`is_hex_color`] is what
/// actually decides, so the two disagreeing costs us a wasted round trip, never
/// a bad write. `schema_pattern_matches_the_validator` guards the pair.
pub const HEX_COLOR_PATTERN: &str = "^#[0-9a-fA-F]{6}$";

/// Whether a string is a `#rrggbb` colour. The enforcement point behind
/// [`HEX_COLOR_PATTERN`]; see there for why the short and alpha forms are out.
pub fn is_hex_color(s: &str) -> bool {
    let Some(digits) = s.strip_prefix('#') else { return false };
    digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The JSON Schema for `kind`'s structured description.
///
/// Every declared section is required. A model that decides a section does not
/// apply and omits it produces a node with a silently blank field, which reads
/// as "the model considered this and had nothing to say" — so we would rather
/// it be asked again.
pub fn description_schema(kind: NodeKind) -> Value {
    let def = kind_def(kind);

    let mut properties = Map::new();
    let mut required = Vec::new();
    for section in def.sections {
        properties.insert(section.key.to_string(), section_schema(section));
        required.push(Value::String(section.key.to_string()));
    }

    json!({
        "type": "object",
        "description": format!(
            "Structured visual description of a {}. Every field is required and must be \
             specific enough to change what an artist would draw.",
            def.label,
        ),
        "properties": Value::Object(properties),
        "required": Value::Array(required),
        // Closing the object is the standard way to say "no other fields"; it is
        // a hint like the rest, since the validator builds its result from the
        // registry and so ignores anything extra whether or not this is honoured.
        "additionalProperties": false,
    })
}

/// One property of the flat object.
///
/// Length and item-count bounds are left out on purpose: Google documents
/// numeric bounds but not string or array ones, and a schema a provider rejects
/// outright is worse than one it under-enforces. Emptiness is caught in
/// `wobu-llm` on the way back instead.
fn section_schema(section: &SectionDef) -> Value {
    let label = section.label;
    match section.value_kind {
        SectionValueKind::Text => json!({
            "type": "string",
            "description": format!("{label}. One short paragraph of visual detail."),
        }),
        // `palette` is the one list whose items have a form, so it is the one
        // list that can carry a pattern. Matched on the key rather than a flag
        // in the registry because it is the only exception, and a `value_kind`
        // per list shape would be a registry change for one field.
        SectionValueKind::List if section.key == PALETTE_KEY => json!({
            "type": "array",
            "description": format!("{label}. Hex colours in #rrggbb form."),
            "items": { "type": "string", "pattern": HEX_COLOR_PATTERN },
        }),
        SectionValueKind::List => json!({
            "type": "array",
            "description": format!("{label}. Short phrases, not sentences."),
            "items": { "type": "string" },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::kind_registry;

    #[test]
    fn every_kind_in_the_registry_gets_a_schema() {
        // The regression this guards: a kind added to the registry without a
        // schema would fail only at enhance time, in front of a user, on that
        // one kind. Nothing else in the codebase would notice.
        for def in kind_registry() {
            let schema = description_schema(def.kind);
            let properties = schema["properties"].as_object().unwrap();
            assert_eq!(
                properties.len(),
                def.sections.len(),
                "{} should have one property per section",
                def.kind
            );
            for section in def.sections {
                assert!(
                    properties.contains_key(section.key),
                    "{} is missing property {}",
                    def.kind,
                    section.key
                );
            }
            let required: Vec<&str> = schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(
                required,
                def.sections.iter().map(|s| s.key).collect::<Vec<_>>(),
                "{} should require every declared section, in declared order",
                def.kind
            );
        }
    }

    #[test]
    fn schemas_stay_flat() {
        // Google rejects deeply nested schemas. No property may be an object, and
        // an array's items must be plain strings — one level, never two.
        for def in kind_registry() {
            let schema = description_schema(def.kind);
            for (key, property) in schema["properties"].as_object().unwrap() {
                match property["type"].as_str().unwrap() {
                    "string" => {}
                    "array" => assert_eq!(
                        property["items"]["type"], "string",
                        "{}.{key} nests a non-string item",
                        def.kind
                    ),
                    other => panic!("{}.{key} is a {other}, which nests", def.kind),
                }
            }
        }
    }

    #[test]
    fn list_sections_are_arrays_and_prose_sections_are_strings() {
        let schema = description_schema(NodeKind::Character);
        assert_eq!(schema["properties"]["silhouette"]["type"], "string");
        assert_eq!(schema["properties"]["palette"]["type"], "array");
        assert_eq!(schema["properties"]["signature"]["type"], "array");
        assert_eq!(schema["properties"]["never"]["type"], "array");
    }

    #[test]
    fn the_only_list_sections_are_palette_signature_and_never() {
        // `section_schema` treats `palette` as the one list with a form and the
        // rest as free phrases. A new list section added to the registry needs a
        // decision about which it is, and this is where that decision is forced.
        let mut lists: Vec<&str> = kind_registry()
            .iter()
            .flat_map(|def| def.sections)
            .filter(|s| s.value_kind == SectionValueKind::List)
            .map(|s| s.key)
            .collect();
        lists.sort_unstable();
        lists.dedup();
        assert_eq!(lists, ["never", PALETTE_KEY, "signature"]);
    }

    #[test]
    fn only_palette_constrains_its_items() {
        // `signature` and `never` are free prose fragments; a pattern on them
        // would reject perfectly good output.
        let schema = description_schema(NodeKind::Character);
        assert_eq!(schema["properties"]["palette"]["items"]["pattern"], HEX_COLOR_PATTERN);
        assert!(schema["properties"]["never"]["items"]["pattern"].is_null());
    }

    #[test]
    fn schema_pattern_matches_the_validator() {
        // The pattern is a hand-maintained copy of `is_hex_color`. If someone
        // widens one to accept `#fff` or alpha, this fails until they widen both.
        assert_eq!(HEX_COLOR_PATTERN, "^#[0-9a-fA-F]{6}$");
        assert!(is_hex_color("#2b2118"));
        assert!(is_hex_color("#C2703A"), "case is not meaningful in hex");
        assert!(!is_hex_color("#fff"), "shorthand is rejected");
        assert!(!is_hex_color("#2b211880"), "alpha is rejected");
        assert!(!is_hex_color("2b2118"), "the hash is required");
        assert!(!is_hex_color("#2b211g"));
        assert!(!is_hex_color(""));
    }

    #[test]
    fn schemas_serialise_to_json_a_provider_would_accept() {
        for def in kind_registry() {
            let text = serde_json::to_string(&description_schema(def.kind)).unwrap();
            assert!(text.contains("\"additionalProperties\":false"), "{}", def.kind);
        }
    }
}
