//! Bounded, credential-free structured authoring transport. Semantic validation
//! remains with the compiler; raw JSON is retained so duplicate keys are visible.
use crate::provider::{DeltaSink, StructuredOutcome, StructuredRequest, Usage};
use crate::stream::{Read, Sse, SseConsumer, next_chunk};
use crate::transport::{self, SseEnhance};
use crate::{Cancel, Error};

pub(crate) const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_WIRE_BYTES: usize = 1024 * 1024;

pub(crate) fn append(
    json: &mut String,
    fragment: &str,
    closed: bool,
    structured: bool,
    deltas: &mut dyn DeltaSink,
) -> crate::Result<()> {
    if structured && (closed || json.len().saturating_add(fragment.len()) > MAX_JSON_BYTES) {
        return Err(Error::NotJson("Closed or oversized structured output".into()));
    }
    json.push_str(fragment);
    deltas.delta(fragment);
    Ok(())
}

/// Deliberately never include provider body text, endpoint URLs or transport
/// error strings in a structured receipt. Those can echo prompts or credentials.
fn safe_error(error: Error) -> Error {
    match error {
        Error::RateLimited { .. } => Error::Unavailable {
            detail: "The provider failed after accepting the request; billing is unknown".into(),
        },
        Error::Unavailable { .. } => Error::Unavailable {
            detail: "The structured request failed; billing is unknown".into(),
        },
        Error::SchemaRejected { .. } => {
            Error::SchemaRejected { detail: "The provider rejected the structured request".into() }
        }
        Error::NotJson(_) => Error::NotJson("Invalid or oversized structured response".into()),
        other => other,
    }
}

fn finish(consumer: impl SseConsumer, aborted: Option<Error>) -> StructuredOutcome {
    let mut outcome = consumer.finish_raw(aborted);
    outcome.result = outcome.result.map_err(safe_error).and_then(|json| {
        // Parse only to check framing/syntax. Never serialize this value back:
        // doing so would erase duplicate keys before the caller's strict parser.
        let value: serde_json::Value = serde_json::from_str(&json)
            .map_err(|_| Error::NotJson("Invalid structured JSON".into()))?;
        if !value.is_object() {
            return Err(Error::NotAnObject { found: crate::error::json_type_name(&value) });
        }
        Ok(json)
    });
    outcome
}

pub(crate) async fn generate<P: SseEnhance>(
    provider: &P,
    request: &StructuredRequest,
    deltas: &mut dyn DeltaSink,
    cancel: &Cancel,
) -> StructuredOutcome {
    let failure = |error| StructuredOutcome { usage: Usage::default(), result: Err(error) };
    if cancel.is_cancelled() {
        return failure(Error::Cancelled);
    }
    if request.max_output_tokens == 0 || !request.schema.is_object() {
        return failure(Error::SchemaRejected {
            detail: "Structured requests require an object schema and a positive output limit"
                .into(),
        });
    }
    let body = match transport::json_body(&P::structured_body(request)) {
        Ok(body) => body,
        Err(error) => return failure(safe_error(error)),
    };
    let send = provider
        .authenticate(provider.client().post(provider.base_url()))
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .body(body);
    let response = match transport::send(send, cancel).await {
        Ok(response) => response,
        Err(transport::Failure::Cancelled) => return failure(Error::Cancelled),
        Err(transport::Failure::Unavailable(_)) => {
            return failure(safe_error(Error::Unavailable { detail: String::new() }));
        }
    };
    if !response.status().is_success() {
        let status = response.status().as_u16();
        // No body read: a gateway can echo credentials, or stream an unbounded
        // error body forever. Only the actual HTTP status establishes rejection.
        let error = P::error_for_status(status, "", transport::retry_after(&response));
        return failure(if status == 429 { error } else { safe_error(error) });
    }
    let mut body = std::pin::pin!(response.bytes_stream());
    let mut consumer = P::structured_consumer();
    let mut sse = Sse::new();
    let mut bytes_seen = 0usize;
    loop {
        match next_chunk(body.as_mut(), cancel).await {
            Read::Chunk(Ok(bytes)) => {
                bytes_seen = bytes_seen.saturating_add(bytes.len());
                if bytes_seen > MAX_WIRE_BYTES {
                    return finish(consumer, Some(Error::NotJson("Wire limit exceeded".into())));
                }
                // Bound total wire bytes before the framing accumulator allocates.
                sse.push(&bytes);
                if sse.has_invalid_utf8() {
                    return finish(
                        consumer,
                        Some(Error::NotJson("Invalid UTF-8 in stream".into())),
                    );
                }
                while let Some(event) = sse.next_event() {
                    if cancel.is_cancelled() {
                        return finish(consumer, Some(Error::Cancelled));
                    }
                    if consumer.event(&event, deltas) {
                        return finish(consumer, None);
                    }
                }
            }
            Read::Chunk(Err(_)) => {
                return finish(consumer, Some(Error::Unavailable { detail: String::new() }));
            }
            Read::End => return finish(consumer, None),
            Read::Cancelled => return finish(consumer, Some(Error::Cancelled)),
        }
    }
}

/// Gemini documents anyOf, not oneOf. String enums and objects are disjoint,
/// so their union has identical semantics. Do not rewrite overlapping unions,
/// an existing anyOf conjunction, or enum/const instance data as though it were schema.
pub(crate) fn gemini_schema(source: &serde_json::Value) -> serde_json::Value {
    let mut schema = source.clone();
    let Some(map) = schema.as_object_mut() else { return schema };
    if !map.contains_key("anyOf")
        && let Some(branches) = map.get("oneOf").and_then(|v| v.as_array())
    {
        let strings = |v: &serde_json::Value| {
            v.get("enum")
                .and_then(|v| v.as_array())
                .is_some_and(|values| !values.is_empty() && values.iter().all(|v| v.is_string()))
        };
        if branches.len() == 2
            && ((strings(&branches[0]) && branches[1]["type"] == "object")
                || (strings(&branches[1]) && branches[0]["type"] == "object"))
        {
            let union = map.remove("oneOf").expect("checked union");
            map.insert("anyOf".into(), union);
        }
    }
    for (key, value) in map.iter_mut() {
        match key.as_str() {
            "properties" | "$defs" | "definitions" | "patternProperties" => {
                if let Some(properties) = value.as_object_mut() {
                    for property in properties.values_mut() {
                        *property = gemini_schema(property);
                    }
                }
            }
            "items" | "additionalProperties" | "not" | "if" | "then" | "else" => {
                *value = gemini_schema(value)
            }
            "oneOf" | "anyOf" | "allOf" | "prefixItems" => {
                if let Some(branches) = value.as_array_mut() {
                    for branch in branches {
                        *branch = gemini_schema(branch);
                    }
                }
            }
            _ => {}
        }
    }
    schema
}

#[cfg(test)]
mod tests {
    use super::gemini_schema;
    use serde_json::json;

    #[test]
    fn dialect_projection_never_relaxes_overlapping_unions_or_edits_instance_data() {
        for schema in [
            json!({"oneOf":[{"type":"string"},{"enum":["player"]}]}),
            json!({"oneOf":[{"enum":["player"]},{"type":"object"}],"anyOf":[{"type":"string"}]}),
            json!({"enum":[{"oneOf":[{"enum":["player"]},{"type":"object"}]}]}),
        ] {
            assert_eq!(gemini_schema(&schema), schema);
        }
    }
}
