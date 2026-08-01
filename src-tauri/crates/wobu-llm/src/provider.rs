//! The text provider trait: one shape that Anthropic and Gemini both fit.
//!
//! Two adapters land at the same time ([#34](https://github.com/krazyjakee/wobu/issues/34),
//! [#35](https://github.com/krazyjakee/wobu/issues/35)) rather than one and then
//! the other, because a trait written against a single vendor is that vendor's
//! request struct wearing a trait, and the second adapter is the one that has to
//! pay for it. So everything here is at the intersection of what both document,
//! and where they genuinely differ the difference stays on the adapter's side of
//! the line. The places that happened are marked below with which vendor forced
//! the shape.
//!
//! What is deliberately *not* here: a message list, a tool name, sampling
//! parameters, an HTTP client, or anything that only makes sense for one of
//! them. Enhance is one question with one answer, and the request says only
//! that.
//!
//! See `docs/08-providers.md`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wobu_core::NodeKind;
use wobu_core::schema::description_schema;

use crate::cancel::Cancel;
use crate::error::{Error, Result};
use crate::validate::ValidatedDescription;

/// How much output to allow when nothing else says.
///
/// A description is a handful of short sections, so this is roomy — deliberately.
/// Output tokens are billed on what is produced, not on the cap, whereas a cap
/// set too tight buys a truncated response that has to be paid for twice.
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 4096;

/// The one property [`EnhanceRequest::schema`] adds to the kind's own schema,
/// and the only thing a model may send back that is not a description section.
///
/// It is here rather than in `wobu_core`'s registry, and that placement is the
/// argument. A section in the registry is *canon*: it is normalised onto the
/// node, written into the Markdown, and extracted as a weighted fragment into
/// every prompt compiled from that entity. "What colour is the guild signet?" is
/// none of those things — it is a property of one call, addressed to the person
/// who wrote the notes, and it stops being true the moment they answer it. So it
/// is declared per *request*, read off the response beside the description
/// ([`crate::ValidatedDescription::questions`]), and has no route to disk at all.
///
/// Declared rather than merely tolerated, even though `wobu_core::schema` makes
/// undeclared fields non-fatal. The schema says `additionalProperties: false`,
/// so a field asked for in prose and refused by the schema is a model told two
/// opposite things — and the one that would give way is the instruction not to
/// confabulate, which is the whole reason the field exists.
pub const QUESTIONS_KEY: &str = "questions";

/// One Enhance call: fill in every section of `kind`'s description.
///
/// The schema is not a field. It is derived from `kind` by [`EnhanceRequest::schema`]
/// and the answer is validated against the same `kind`, so there is no way to
/// send one shape and check another — which is the failure that would look
/// exactly like a provider ignoring the schema it was given.
#[derive(Debug, Clone)]
pub struct EnhanceRequest {
    /// Decides both the schema sent and the sections required back.
    pub kind: NodeKind,
    /// Provider-specific and opaque to this crate: it comes from `project.json`
    /// and means nothing without the adapter that reads it. Model ids move
    /// faster than anything else in `docs/08-providers.md`, which is the reason
    /// there is no enum of them here.
    pub model: String,
    /// Standing instructions — who the model is being for this call.
    ///
    /// Kept apart from `prompt` because Anthropic has a top-level `system` field
    /// and putting it in the user turn measurably weakens it. Gemini's
    /// Interactions API equivalent is unconfirmed (🚩 in `docs/08-providers.md`),
    /// so that adapter may end up folding this into its `input`. That is the
    /// adapter's call to make; a request shaped around either answer would force
    /// the other one to unpick it.
    pub system: Option<String>,
    /// The compiled prompt: what to describe, and everything the influence stack
    /// contributed. One turn, no history — Enhance never converses, and a
    /// `Vec<Message>` here would be Anthropic's role model leaking through a
    /// trait Gemini also has to fit.
    pub prompt: String,
    /// Required, not optional, because Anthropic's `max_tokens` is required and
    /// an adapter inventing a default would mean the same request costs a
    /// different ceiling depending on which provider is selected.
    pub max_output_tokens: u32,
}

impl EnhanceRequest {
    pub fn new(kind: NodeKind, model: impl Into<String>, prompt: impl Into<String>) -> Self {
        EnhanceRequest {
            kind,
            model: model.into(),
            system: None,
            prompt: prompt.into(),
            max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    pub fn with_max_output_tokens(mut self, max_output_tokens: u32) -> Self {
        self.max_output_tokens = max_output_tokens;
        self
    }

    /// The JSON Schema the answer must satisfy.
    ///
    /// This is the request's whole reason for existing in this form: structured
    /// output means handing the provider a schema, not a prompt asking politely
    /// for JSON. Both vendors accept one, and both accept *this* one — it is
    /// generated flat from the kind registry precisely so it sits inside the
    /// subset Google documents while remaining steerable as Anthropic tool input
    /// (`wobu_core::schema`).
    ///
    /// What each adapter wraps it in is theirs: Anthropic needs a tool with a
    /// name and a `tool_choice` pinning it, Gemini needs a `response_format`
    /// with a mime type and nothing else. The name Anthropic requires is an
    /// invention of that adapter and has no business being in a request Gemini
    /// also has to read.
    ///
    /// [`QUESTIONS_KEY`] is added here and *not* to `required`: a model with
    /// nothing to ask omits it, which is the common case and must not be a
    /// missing-section failure. See there for why it is not a section.
    pub fn schema(&self) -> Value {
        let mut schema = description_schema(self.kind);
        if let Some(properties) = schema["properties"].as_object_mut() {
            properties.insert(
                QUESTIONS_KEY.to_string(),
                json!({
                    "type": "array",
                    "description": "Facts the notes do not settle and that you would \
                                    otherwise have had to invent. One short question each, \
                                    addressed to the person who wrote the notes. Leave this \
                                    out when the notes settle everything.",
                    "items": { "type": "string" },
                }),
            );
        }
        schema
    }
}

/// Where streamed output goes on its way to the editor.
///
/// A callback rather than a returned stream, and the reasoning is worth keeping:
///
/// - **A stream would carry the description.** If the caller assembles the
///   answer from the items it pulls, then a stream that ends early yields a
///   description that looks finished, and the failure is one `while let` away
///   from being ignored. Here the only route to a [`ValidatedDescription`] is
///   [`EnhanceOutcome::result`], so a partial response cannot be mistaken for a
///   complete one — the partial text went somewhere else entirely.
/// - **Usage cannot be skipped past.** Same argument: the return value is not a
///   `Result`, so the tokens survive whatever the call did.
/// - **Cancellation stays explicit.** A stream would tempt "cancel by dropping
///   it", which is invisible at the call site and, for an adapter that spawned a
///   task, does not stop anything. [`Cancel`] is a thing the caller holds and
///   the adapter must honour.
/// - The consumer is a Tauri event emitter (`enhance:delta`), which is
///   fire-and-forget. There is no combinator pipeline here to justify the
///   `Pin<Box<dyn Stream>>` a `dyn TextProvider` would need.
///
/// A trait rather than a bare `FnMut` so it can gain a defaulted method later —
/// incremental usage for a live spend meter is the likely one — without every
/// adapter signature changing. Closures implement it, so callers rarely notice.
pub trait DeltaSink: Send {
    /// A fragment of the response document, in order. Concatenating every
    /// fragment of a completed call gives exactly the text that was validated.
    ///
    /// **This is JSON, not prose.** Both vendors stream the structured document
    /// itself — Anthropic as `input_json_delta` on the tool block, Gemini as
    /// partial text of the JSON body — so there is no point in the stream where
    /// a whole section exists as a string. Promising `(section, text)` deltas
    /// would mean an incremental JSON reader inside every adapter, identical in
    /// each; that reader belongs in this crate, above the trait, and the editor
    /// gets its typing effect from it rather than from here.
    fn delta(&mut self, json: &str);
}

impl<F: FnMut(&str) + Send> DeltaSink for F {
    fn delta(&mut self, json: &str) {
        self(json)
    }
}

/// A sink for callers that want the answer without the typing effect — a
/// batch re-enhance, a test. Named rather than `|_| {}` so the intent reads.
pub struct Discard;

impl DeltaSink for Discard {
    fn delta(&mut self, _json: &str) {}
}

/// What a provider charged, whatever the call did afterwards.
///
/// Zero means "nothing we know of was billed", which is not the same as "nothing
/// was billed": a call cancelled before the provider reported anything may still
/// have run. Adapters report the last figures they saw rather than waiting for a
/// clean finish, because a request that is cancelled or truncated has been paid
/// for either way and [#55](https://github.com/krazyjakee/wobu/issues/55)'s
/// spend ceiling is only a ceiling if it counts those.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u32,
    /// Input served from a provider-side cache. Split out rather than folded
    /// into `input_tokens` because both vendors price it differently from fresh
    /// input — an order of magnitude, in Anthropic's case — and a spend estimate
    /// that ignores that is wrong in the direction that scares people.
    pub cached_input_tokens: u32,
    pub output_tokens: u32,
}

impl Usage {
    /// Saturating, because a corrupt figure from a provider should not panic a
    /// running job — and a spend meter pinned at the maximum is a visible bug,
    /// where a wrapped one is an invisible one.
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens.saturating_add(self.cached_input_tokens).saturating_add(self.output_tokens)
    }
}

/// The result of an Enhance call: what it cost, and what came back.
///
/// Not a `Result`, on purpose. `?` on a `Result<_, E>` would carry the error out
/// and leave the usage behind, and "the call failed so nothing was charged" is
/// false often enough — a rate limit mid-stream, a cancellation, a truncation —
/// that the spend ceiling would drift low precisely when the user is hitting
/// limits. Destructuring is the only way past this type, and destructuring puts
/// [`Usage`] in front of whoever wrote the call.
#[derive(Debug)]
pub struct EnhanceOutcome {
    pub usage: Usage,
    /// `Ok` only for a response that arrived whole and passed the kind's schema.
    /// There is no partial value: whatever text a failed call streamed went to
    /// the [`DeltaSink`] and stops there, so nothing that could be mistaken for
    /// a description survives a failure.
    pub result: Result<ValidatedDescription>,
}

impl EnhanceOutcome {
    pub fn new(usage: Usage, result: Result<ValidatedDescription>) -> Self {
        EnhanceOutcome { usage, result }
    }

    /// A failure before the provider could have charged anything: no key, a
    /// refused connection, a cancellation that beat the request out of the door.
    /// Named so that reaching for it is a claim about billing rather than a
    /// convenience — anything that got as far as a response body should be
    /// reporting real figures through [`EnhanceOutcome::new`].
    pub fn unbilled(error: Error) -> Self {
        EnhanceOutcome { usage: Usage::default(), result: Err(error) }
    }

    pub fn is_ok(&self) -> bool {
        self.result.is_ok()
    }
}

/// A text provider, selected per project and held behind a `dyn`.
///
/// `#[async_trait]` rather than a native `async fn`: the provider is chosen at
/// runtime from `project.json`, so this is stored as `Box<dyn TextProvider>`,
/// and a native async trait method gives no `Send` future to put in a box.
/// Nothing here names a runtime — Tauri's is the one it will run on, and the job
/// queue owns retries and concurrency.
#[async_trait]
pub trait TextProvider: Send + Sync {
    /// Stable id: the `provider` in `project.json` and the `wobu/<provider>`
    /// entry in the OS keychain. Renaming one breaks both.
    fn id(&self) -> &'static str;

    /// The name a person sees, including inside error messages built here —
    /// which is why it is on the trait and not left to the UI.
    fn label(&self) -> &'static str;

    /// Used when a project names this provider but no model. Kept with the
    /// adapter because model ids are the fastest-moving fact in
    /// `docs/08-providers.md` and should not be spelled out in the frontend.
    fn default_model(&self) -> &'static str;

    /// Ask for one description.
    ///
    /// Contract for implementors, all three parts of it load-bearing:
    ///
    /// 1. Every fragment of the response document goes to `deltas`, in order,
    ///    as it arrives.
    /// 2. `cancel` is honoured by *stopping the request* — dropping the response
    ///    body so the connection closes — not by running to completion and
    ///    discarding the answer. An abandoned request keeps generating tokens the
    ///    user pays for. Racing the next read against [`Cancel::cancelled`] is
    ///    the shape this is designed for; polling [`Cancel::check`] between
    ///    chunks alone leaves the user paying for however long a quiet provider
    ///    takes to say the next word.
    /// 3. The returned [`Usage`] is the best figure known at the moment the call
    ///    ended, success or not.
    ///
    /// A response that arrived whole is validated with
    /// [`crate::validate::parse_description`] before it is returned, no matter
    /// what the provider promises about schema conformance. A response known to
    /// be cut short is [`Error::Truncated`] and is never handed to the validator
    /// hoping it will notice.
    async fn enhance(
        &self,
        request: &EnhanceRequest,
        deltas: &mut dyn DeltaSink,
        cancel: &Cancel,
    ) -> EnhanceOutcome;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `schema()` minus the one property it adds, so a test can compare against
    /// the registry's answer without restating what the addition is.
    fn sections_only(mut schema: Value) -> Value {
        schema["properties"].as_object_mut().unwrap().remove(QUESTIONS_KEY);
        schema
    }

    #[test]
    fn a_request_carries_the_schema_for_its_kind_rather_than_a_prompt_asking_for_json() {
        // Structured output is the point of the trait. If this ever came back
        // empty, every adapter would still "work" and every response would be
        // whatever prose the model felt like.
        let request = EnhanceRequest::new(NodeKind::Character, "test-model", "Describe Vashk.");
        let schema = request.schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["palette"].is_object());
        assert_eq!(sections_only(schema), description_schema(NodeKind::Character));
    }

    #[test]
    fn the_schema_follows_the_kind_so_the_two_cannot_be_set_separately() {
        // The regression: a request built for a Character and validated as a
        // Setting would read as the provider ignoring its schema, and the fix
        // would be looked for in the adapter.
        for kind in [NodeKind::Character, NodeKind::Setting, NodeKind::Prop] {
            let request = EnhanceRequest::new(kind, "test-model", "…");
            assert_eq!(request.kind, kind);
            assert_eq!(sections_only(request.schema()), description_schema(kind));
        }
    }

    #[test]
    fn every_kind_may_ask_a_question_and_no_kind_is_required_to() {
        // "Ask rather than confabulate" is only an instruction the model can
        // follow if the shape it was handed has somewhere to put the question —
        // and a `questions` the schema required would turn "the notes settle
        // everything" into a wasted call.
        for def in wobu_core::kind::kind_registry() {
            let schema = EnhanceRequest::new(def.kind, "test-model", "…").schema();
            assert_eq!(
                schema["properties"][QUESTIONS_KEY]["items"]["type"],
                "string",
                "{} cannot be asked a question",
                def.kind,
            );
            let required: Vec<&str> =
                schema["required"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
            assert!(!required.contains(&QUESTIONS_KEY), "{} is forced to ask one", def.kind);
            // And the object is still closed, so `questions` is the only thing
            // a model is invited to send that is not a section.
            assert_eq!(schema["additionalProperties"], false, "{}", def.kind);
        }
    }

    #[test]
    fn a_request_has_an_output_ceiling_even_when_nobody_set_one() {
        // Anthropic rejects a request without `max_tokens`, so a default that
        // only appeared in one adapter would be a difference in cost between
        // providers that nothing in the UI explains.
        let request = EnhanceRequest::new(NodeKind::Character, "test-model", "…");
        assert_eq!(request.max_output_tokens, DEFAULT_MAX_OUTPUT_TOKENS);
        assert_eq!(request.with_max_output_tokens(512).max_output_tokens, 512);
    }

    #[test]
    fn usage_totals_without_wrapping() {
        let usage = Usage { input_tokens: 900, cached_input_tokens: 100, output_tokens: 40 };
        assert_eq!(usage.total_tokens(), 1040);
        let absurd = Usage { input_tokens: u32::MAX, cached_input_tokens: 5, output_tokens: 5 };
        assert_eq!(absurd.total_tokens(), u32::MAX);
    }

    #[test]
    fn an_unbilled_failure_says_zero_rather_than_saying_nothing() {
        // `usage` is not optional, so "we do not know" and "nothing was charged"
        // have to be the same value; the spend ceiling reads it either way.
        let outcome = EnhanceOutcome::unbilled(Error::NoKey { provider: "Anthropic" });
        assert_eq!(outcome.usage, Usage::default());
        assert!(!outcome.is_ok());
    }

    #[test]
    fn a_closure_is_a_delta_sink() {
        // Adapters take `&mut dyn DeltaSink`; if the everyday caller had to
        // define a type for it, tests and one-off calls would grow a wrapper
        // each.
        let mut seen = String::new();
        let mut sink = |json: &str| seen.push_str(json);
        let dynamic: &mut dyn DeltaSink = &mut sink;
        dynamic.delta("{\"silhouette\":");
        dynamic.delta("\"tall\"}");
        assert_eq!(seen, "{\"silhouette\":\"tall\"}");
    }
}
