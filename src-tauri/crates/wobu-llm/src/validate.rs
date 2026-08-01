//! Checking a provider's answer against the kind's schema before it becomes canon.
//!
//! This runs on every response no matter what the provider promises. Anthropic
//! tool-use and Google's `response_format` both claim schema-valid output and
//! both sometimes miss — an omitted field, a list where prose was asked for, a
//! palette entry spelled `dark ochre`. The cost of believing them is malformed
//! data written into a node and compiled into every prompt after that, so the
//! guarantee is treated as a hint and this module is the actual gate.

use serde_json::Value;
use wobu_core::kind::{NodeKind, SectionValueKind, kind_def};
use wobu_core::schema::{PALETTE_KEY, is_hex_color};
use wobu_core::{Description, SectionValue};

use crate::error::{Error, Result, json_type_name};
use crate::provider::QUESTIONS_KEY;

/// A response that passed. `extra_sections` is not a failure — see
/// [`validate_description`].
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedDescription {
    /// Sections in the kind's declared order, ready to write to a node.
    pub description: Description,
    /// What the model would have had to invent, asked instead.
    ///
    /// Beside the description rather than inside it, because that is the whole
    /// of the argument in [`QUESTIONS_KEY`]: these are addressed to the person
    /// who wrote the notes, they stop being true once answered, and nothing here
    /// may reach the node. Empty is the ordinary case and means the notes
    /// settled everything — not that the model was not asked.
    pub questions: Vec<String>,
    /// Sections the model volunteered that this kind does not declare. Dropped
    /// from `description`, kept here so a provider that has started inventing
    /// fields shows up in a log rather than only in a shrug. Order follows the
    /// decoded response's key order, which `serde_json` sorts — do not read
    /// anything into it.
    pub extra_sections: Vec<String>,
}

/// Parse raw provider text and validate it.
///
/// The text is expected to be bare JSON, which is what both tool-use and
/// structured-output modes return. A model that wraps its answer in a markdown
/// fence has ignored the request format, and that is a retry, not something to
/// paper over here.
pub fn parse_description(kind: NodeKind, raw: &str) -> Result<ValidatedDescription> {
    let value: Value = serde_json::from_str(raw).map_err(|e| Error::NotJson(e.to_string()))?;
    validate_description(kind, &value)
}

/// Validate a decoded response against `kind`'s schema.
///
/// The result is built by walking the registry's sections rather than the
/// response's keys, which is what makes the two failure modes asymmetric:
///
/// - A **missing** section is fatal. Enhance is meant to fill every section the
///   kind declares; a node saved with one silently blank reads as "the model
///   considered this and had nothing to say", and `never` in particular drives
///   the negative prompt for every generation off that node.
/// - An **extra** section is not. It costs nothing to ignore, `Description` is
///   normalised to the declared set on read anyway, and failing the whole
///   response over a field we were going to discard would burn a paid call for
///   no gain.
pub fn validate_description(kind: NodeKind, response: &Value) -> Result<ValidatedDescription> {
    let object =
        response.as_object().ok_or(Error::NotAnObject { found: json_type_name(response) })?;

    let declared = kind_def(kind).sections;
    let mut sections = Vec::with_capacity(declared.len());
    for def in declared {
        // A JSON `null` is how several providers spell "I had nothing here". It
        // is the same outcome as omitting the key, so it gets the same error.
        let value = match object.get(def.key) {
            None | Some(Value::Null) => {
                return Err(Error::MissingSection { section: def.key });
            }
            Some(value) => value,
        };
        let section = match def.value_kind {
            SectionValueKind::Text => text_section(def.key, value)?,
            SectionValueKind::List => list_section(def.key, value)?,
        };
        sections.push((def.key.to_string(), section));
    }

    let extra_sections = object
        .keys()
        .filter(|key| key.as_str() != QUESTIONS_KEY)
        .filter(|key| !declared.iter().any(|def| def.key == key.as_str()))
        .cloned()
        .collect();

    Ok(ValidatedDescription {
        description: Description::from_sections(sections),
        questions: questions(object.get(QUESTIONS_KEY)),
        extra_sections,
    })
}

/// The questions, read as leniently as anything here is read.
///
/// A malformed `questions` is never a failure, and that asymmetry with the
/// sections above is deliberate: the description is what becomes canon and is
/// worth another paid call to get right, whereas a question is a note in the
/// margin. Burning a call the model otherwise answered perfectly, because it put
/// a number in the list it was not obliged to send at all, would be the app
/// spending the user's money on tidiness.
fn questions(value: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = value else { return Vec::new() };
    items
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(str::to_owned)
        .collect()
}

fn text_section(key: &'static str, value: &Value) -> Result<SectionValue> {
    let text = value.as_str().ok_or(Error::WrongSectionType {
        section: key,
        expected: "a string",
        found: json_type_name(value),
    })?;
    // Trimmed on the way in because providers pad with leading newlines, and the
    // Markdown writer would turn that padding into blank lines that then look
    // like a hand edit on the next read.
    let text = text.trim();
    if text.is_empty() {
        // Whitespace-only is not a shorter answer, it is a non-answer, and it
        // would leave the section rendering blank in the editor while the node
        // claimed to be freshly enhanced.
        return Err(Error::EmptySection { section: key });
    }
    Ok(SectionValue::Text(text.to_string()))
}

fn list_section(key: &'static str, value: &Value) -> Result<SectionValue> {
    let array = value.as_array().ok_or(Error::WrongSectionType {
        section: key,
        expected: "an array of strings",
        found: json_type_name(value),
    })?;

    let mut items = Vec::with_capacity(array.len());
    for item in array {
        let item = item.as_str().ok_or(Error::WrongSectionType {
            section: key,
            expected: "an array of strings",
            found: json_type_name(item),
        })?;
        let item = item.trim();
        // A blank entry is a formatting artifact — a trailing comma's worth of
        // nothing — so it is dropped rather than failing the response. If they
        // were all blank the emptiness check below still catches it.
        if item.is_empty() {
            continue;
        }
        if key == PALETTE_KEY && !is_hex_color(item) {
            // Not dropped like a blank: a colour name or a malformed hex is
            // something the model meant, and swallowing it would leave the
            // palette quietly short of the colours the description talks about.
            return Err(Error::NotAHexColor { section: key, value: item.to_string() });
        }
        items.push(item.to_string());
    }

    if items.is_empty() {
        return Err(Error::EmptySection { section: key });
    }
    Ok(SectionValue::List(items))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A response that satisfies `kind`'s schema, so each test can bend exactly
    /// one thing and know that is what failed.
    fn valid_response(kind: NodeKind) -> Value {
        let mut object = serde_json::Map::new();
        for def in kind_def(kind).sections {
            let value = match (def.value_kind, def.key) {
                (SectionValueKind::Text, _) => json!("Tall, narrow-shouldered, forward-canted"),
                (SectionValueKind::List, PALETTE_KEY) => json!(["#2b2118", "#c2703a"]),
                (SectionValueKind::List, _) => json!(["Ember-lit throat vents"]),
            };
            object.insert(def.key.to_string(), value);
        }
        Value::Object(object)
    }

    #[test]
    fn a_well_formed_response_becomes_a_description_in_declared_order() {
        let validated =
            validate_description(NodeKind::Character, &valid_response(NodeKind::Character))
                .unwrap();
        let keys: Vec<&str> = validated.description.sections.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            kind_def(NodeKind::Character).sections.iter().map(|s| s.key).collect::<Vec<_>>(),
        );
        assert!(validated.extra_sections.is_empty());
    }

    #[test]
    fn a_missing_section_is_rejected_rather_than_written_as_a_gap() {
        // The regression: silently accepting a partial response would stamp the
        // node `fresh` with a blank section nobody would think to re-run.
        let mut response = valid_response(NodeKind::Character);
        response.as_object_mut().unwrap().remove("never");
        assert!(matches!(
            validate_description(NodeKind::Character, &response),
            Err(Error::MissingSection { section: "never" })
        ));
    }

    #[test]
    fn a_null_section_is_treated_as_missing() {
        let mut response = valid_response(NodeKind::Character);
        response["anatomy"] = Value::Null;
        assert!(matches!(
            validate_description(NodeKind::Character, &response),
            Err(Error::MissingSection { section: "anatomy" })
        ));
    }

    #[test]
    fn an_extra_section_is_reported_but_does_not_fail_the_response() {
        // Undeclared sections are dropped on read anyway, so rejecting one would
        // waste a whole call over a field we never intended to keep.
        let mut response = valid_response(NodeKind::Character);
        response["climate"] = json!("does not belong on a character");
        response["era"] = json!("nor does this");

        let validated = validate_description(NodeKind::Character, &response).unwrap();
        assert!(!validated.description.sections.contains_key("climate"));
        let mut extra = validated.extra_sections;
        extra.sort();
        assert_eq!(extra, vec!["climate", "era"]);
    }

    #[test]
    fn a_question_comes_back_beside_the_description_and_never_inside_it() {
        // The placement is the decision. A question that reached
        // `description.sections` would be normalised onto the node, written into
        // the Markdown, and extracted as a fragment into every prompt compiled
        // from that entity — so "what colour is the guild signet?" would end up
        // in the text handed to an image backend.
        let mut response = valid_response(NodeKind::Character);
        response[crate::provider::QUESTIONS_KEY] = json!([
            "What does the guild signet look like?",
            "  ",
            "Is the longcoat waxed or oiled?",
        ]);

        let validated = validate_description(NodeKind::Character, &response).unwrap();
        assert_eq!(
            validated.questions,
            ["What does the guild signet look like?", "Is the longcoat waxed or oiled?",]
        );
        assert!(!validated.description.sections.contains_key("questions"));
        // Not an oddity to be reported either — it is a field we asked for.
        assert!(validated.extra_sections.is_empty(), "{:?}", validated.extra_sections);
    }

    #[test]
    fn a_response_with_nothing_to_ask_is_the_ordinary_one() {
        let validated =
            validate_description(NodeKind::Character, &valid_response(NodeKind::Character))
                .unwrap();
        assert!(validated.questions.is_empty());
    }

    #[test]
    fn a_malformed_questions_field_costs_a_shrug_rather_than_a_second_paid_call() {
        // The asymmetry with the sections above, stated. `questions` is optional
        // and marginal; failing a description the model got right because the
        // list it volunteered has a number in it would be spending the user's
        // money on tidiness.
        for bad in [json!("just one, as prose"), json!([7, "and a real one"]), json!(null)] {
            let mut response = valid_response(NodeKind::Character);
            response[crate::provider::QUESTIONS_KEY] = bad.clone();
            let validated = validate_description(NodeKind::Character, &response)
                .unwrap_or_else(|e| panic!("{bad} failed the whole response: {e}"));
            assert!(validated.description.sections.contains_key("never"));
        }
    }

    #[test]
    fn a_section_of_the_wrong_type_is_rejected() {
        let mut response = valid_response(NodeKind::Character);
        response["silhouette"] = json!(["a list where prose was asked for"]);
        assert!(matches!(
            validate_description(NodeKind::Character, &response),
            Err(Error::WrongSectionType { section: "silhouette", found: "an array", .. })
        ));

        let mut response = valid_response(NodeKind::Character);
        response["never"] = json!("prose where a list was asked for");
        assert!(matches!(
            validate_description(NodeKind::Character, &response),
            Err(Error::WrongSectionType { section: "never", found: "a string", .. })
        ));
    }

    #[test]
    fn a_non_string_list_item_is_rejected() {
        let mut response = valid_response(NodeKind::Character);
        response["never"] = json!(["Modern firearms", 7]);
        assert!(matches!(
            validate_description(NodeKind::Character, &response),
            Err(Error::WrongSectionType { section: "never", found: "a number", .. })
        ));
    }

    #[test]
    fn whitespace_only_prose_is_a_failure_not_a_short_answer() {
        let mut response = valid_response(NodeKind::Character);
        response["anatomy"] = json!("   \n  ");
        assert!(matches!(
            validate_description(NodeKind::Character, &response),
            Err(Error::EmptySection { section: "anatomy" })
        ));
    }

    #[test]
    fn prose_is_trimmed_but_not_otherwise_rewritten() {
        let mut response = valid_response(NodeKind::Character);
        response["anatomy"] = json!("\n  Four-jointed digitigrade legs.  \n");
        let validated = validate_description(NodeKind::Character, &response).unwrap();
        assert_eq!(validated.description.text("anatomy"), Some("Four-jointed digitigrade legs."));
    }

    #[test]
    fn blank_list_items_are_dropped_but_an_all_blank_list_fails() {
        let mut response = valid_response(NodeKind::Character);
        response["never"] = json!(["Modern firearms", "  ", ""]);
        let validated = validate_description(NodeKind::Character, &response).unwrap();
        assert_eq!(validated.description.never(), ["Modern firearms"]);

        let mut response = valid_response(NodeKind::Character);
        response["never"] = json!(["", "   "]);
        assert!(matches!(
            validate_description(NodeKind::Character, &response),
            Err(Error::EmptySection { section: "never" })
        ));

        let mut response = valid_response(NodeKind::Character);
        response["never"] = json!([]);
        assert!(matches!(
            validate_description(NodeKind::Character, &response),
            Err(Error::EmptySection { section: "never" })
        ));
    }

    #[test]
    fn palette_entries_must_be_hex_colours() {
        // A colour name reaching the palette would render as an empty swatch and
        // silently weaken the colour-conditioning pass.
        for bad in ["dark ochre", "#fff", "#2b211880", "2b2118", "rgb(0,0,0)"] {
            let mut response = valid_response(NodeKind::Character);
            response["palette"] = json!(["#2b2118", bad]);
            assert!(
                matches!(
                    validate_description(NodeKind::Character, &response),
                    Err(Error::NotAHexColor { section: "palette", .. })
                ),
                "{bad} should be rejected"
            );
        }
    }

    #[test]
    fn a_response_that_is_not_an_object_is_rejected() {
        assert!(matches!(
            validate_description(NodeKind::Character, &json!(["silhouette"])),
            Err(Error::NotAnObject { found: "an array" })
        ));
    }

    #[test]
    fn text_that_is_not_json_is_a_retryable_failure_rather_than_a_panic() {
        let err =
            parse_description(NodeKind::Character, "Sure! Here is the description:").unwrap_err();
        assert!(matches!(err, Error::NotJson(_)));
        assert!(err.is_retryable());
    }

    #[test]
    fn every_schema_violation_is_retryable() {
        // The whole point of validating: a bad response costs another call, never
        // a bad write.
        let mut response = valid_response(NodeKind::Character);
        response.as_object_mut().unwrap().remove("palette");
        assert!(validate_description(NodeKind::Character, &response).unwrap_err().is_retryable());
    }
}
