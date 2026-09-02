//! Provider API keys, and proving one works before it is relied on.
//!
//! The keys themselves live in the OS keychain or Wobu's private app-data
//! fallback (`crate::keys`); what is here is the command surface over them plus
//! the probe. The probe sends the smallest
//! real request the provider will accept rather than hitting a `/models`
//! endpoint, because a key that lists models and cannot generate is exactly the
//! failure the Settings panel exists to catch.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;
// Aliased because the command below has to *be* called `kind_registry` —
// Tauri v2 derives the invoke name from the function name, with no rename.
use wobu_core::NodeKind;
use wobu_llm::{
    AnthropicProvider, Cancel, Discard, EnhanceOutcome, EnhanceRequest, GeminiProvider,
    TextProvider, Usage, anthropic, gemini,
};

use crate::error::{Code, CommandResult, WobuError};
use crate::keys::{KeyRemoval, KeyStatus, Keys, Secret};

/// Whether this machine has a key for each of these providers.
///
/// Presence, never value — `keys.rs` says why, and there is no command anywhere
/// that returns key material. A list rather than one provider at a time because
/// the pane that renders these renders every row at once, and a call per row
/// would be a credential-store round trip per row.
///
/// A machine with no keychain, or a locked one, is an ordinary machine. Existing
/// fallback keys resolve and missing keys remain addable. The `Result` only
/// preserves an unexpected failure of the blocking worker.
#[tauri::command]
pub async fn provider_key_status(
    keys: State<'_, Keys>,
    providers: Vec<String>,
) -> CommandResult<Vec<KeyStatus>> {
    keys.statuses(providers).await
}

/// Store a key for a provider.
///
/// The one command that carries key material, and it carries it *inwards*: the
/// user pasted it into a field, so it is already in the webview and the only
/// question is where it goes next. Nothing sends one back.
///
/// The argument is never logged. `WobuError::new` and `diag` both scrub, so even
/// a mistake here would be masked rather than published — but the rule is that
/// nothing in this function mentions `key` at all.
#[tauri::command]
pub async fn provider_key_set(
    keys: State<'_, Keys>,
    provider: String,
    key: String,
) -> CommandResult<KeyStatus> {
    keys.set(provider, key).await
}

/// Remove this machine's stored key for a provider.
///
/// The result tells "removed" and "there was nothing to remove" apart, because
/// they are different sentences and only one of them is worth showing.
#[tauri::command]
pub async fn provider_key_delete(
    keys: State<'_, Keys>,
    provider: String,
) -> CommandResult<KeyRemoval> {
    keys.delete(provider).await
}

/* ── the provider selection ───────────────────────────────────────────────── */

/// The one node kind the probe asks about.
///
/// A prop has the shortest description in the registry, so it is the cheapest
/// schema to hand a provider and the fastest thing for one to start answering.
const PROBE_KIND: NodeKind = NodeKind::Prop;

/// Deliberately trivial, and deliberately not about anybody's world. The probe
/// asks a real question because a provider only reveals whether it will take our
/// schema by being given it.
const PROBE_PROMPT: &str = "A plain iron nail. One short line per section.";

/// How much of the answer to let the provider produce.
///
/// This is the whole trick that makes the check free enough to offer. Everything
/// the probe is there to find out — the key is accepted, the model id exists for
/// this account, the description schema is one the provider will take, and the
/// model has started emitting the structured document — is settled in the first
/// few tokens. Letting the answer finish would buy nothing except a bill.
const PROBE_MAX_OUTPUT_TOKENS: u32 = 24;

/// What a probe found out, in the terms the Settings pane renders.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub provider: String,
    /// The model actually asked about — the adapter's own default when the
    /// project named none, which is the fact a user has no other way to learn.
    pub model: String,
    pub ok: bool,
    /// One sentence for the pane. On success it says what was *proved*, because
    /// a green tick beside a key field is a claim the user cannot check.
    pub message: String,
    /// The stable dotted code, so a rejected key can be shown differently from
    /// a provider that is having an outage. `None` when the probe passed.
    pub code: Option<String>,
    /// What the check cost, as the provider reported it. Returned rather than
    /// assumed: a button that spends money silently is the thing this pane
    /// exists to prevent.
    pub usage: Usage,
}

/// Check a stored key against the provider it belongs to, at key-entry time.
///
/// The point is *when* it runs. Without this, a mistyped key is discovered by
/// pressing Enhance on a node and watching a job fail — the failure surfaces at
/// generate time, in a place that has nothing to do with credentials, and often
/// on somebody else's machine. Here it surfaces beside the field that caused it.
///
/// What it proves, in order: this machine has a key; the provider accepts it;
/// the model id resolves for this account; the description schema is one the
/// provider will take at all (Google documents a subset of JSON Schema, so this
/// is a real answer and not a formality); and the model begins emitting the
/// structured document. What it does *not* prove is that a full description
/// validates — the answer is cut off at [`PROBE_MAX_OUTPUT_TOKENS`] on purpose,
/// and pretending otherwise would mean charging for a description nobody asked
/// for every time a user pastes a key.
///
/// A provider failure is a *result*, not a rejection: "Anthropic says this key
/// is wrong" is the answer the pane asked for and belongs beside the field, not
/// in a toast. Only the two things that mean the probe could not run at all —
/// no key on this machine, a provider this build does not have — come back as
/// errors.
#[tauri::command]
pub async fn provider_probe(
    keys: State<'_, Keys>,
    provider: String,
    model: Option<String>,
) -> CommandResult<ProbeResult> {
    let secret = keys.secret(&provider).await?.ok_or_else(|| {
        WobuError::new(
            Code::ProviderNoKey,
            "There is no key for this provider on this machine, so there is nothing to check.",
        )
    })?;
    let adapter = probe_provider(&provider, &secret)?;
    let model = model
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| adapter.default_model().to_owned());

    let request = EnhanceRequest::new(PROBE_KIND, &model, PROBE_PROMPT)
        .with_max_output_tokens(PROBE_MAX_OUTPUT_TOKENS);
    // A fresh token that nothing holds: the probe is a few hundred milliseconds
    // and there is no Stop button in Settings to wire one to.
    let outcome = adapter.enhance(&request, &mut Discard, &Cancel::new()).await;

    Ok(verdict(adapter.as_ref(), model, outcome))
}

/// Read an `EnhanceOutcome` as an answer about the key rather than as an answer
/// about a nail.
pub(super) fn verdict(
    adapter: &dyn TextProvider,
    model: String,
    outcome: EnhanceOutcome,
) -> ProbeResult {
    let label = adapter.label();
    let usage = outcome.usage;
    let (ok, message, code) = match outcome.result {
        // Only reachable if a provider fits a whole description into the token
        // ceiling above, which no current one does — but it is the strongest
        // possible pass and reporting it as a failure would be absurd.
        Ok(_) => (true, format!("{label} answered with a complete description."), None),
        Err(error) => {
            let code = error.code();
            // `wobu_llm::Error` is split into "the call" and "the answer", and
            // every variant on the answer side lands on this one code. Reaching
            // the answer side at all means the key, the model id and the schema
            // were all accepted — the request got as far as generating — which
            // is precisely what the probe set out to establish. Matching on the
            // code rather than on the variants keeps this from having to be
            // revisited every time one is added.
            if code == "provider.bad_response" {
                (
                    true,
                    format!(
                        "{label} took the key and started writing with {model}. The check stops \
                         the answer after a few tokens, so it did not finish one."
                    ),
                    None,
                )
            } else {
                (false, error.to_string(), Some(code.to_owned()))
            }
        }
    };
    ProbeResult { provider: adapter.id().to_owned(), model, ok, message, code, usage }
}

/// The text adapters this build has, by the id `project.json` and the keychain
/// both use.
///
/// A second construction site — `enhance.rs` has the same match, private to
/// itself — and the duplication is deliberate rather than overlooked: the
/// modules do not export to each other and the probe must not be the reason
/// `enhance.rs` grows a public surface. What has to stay true is the *set of
/// ids*, and both sides read those from `anthropic::ID` and `gemini::ID` rather
/// than spelling them out, so an adapter added to one and not the other is a
/// probe that cannot check the provider Enhance would actually run.
fn probe_provider(id: &str, key: &Secret) -> CommandResult<Arc<dyn TextProvider>> {
    let built = match id {
        anthropic::ID => {
            AnthropicProvider::new(key.expose()).map(|p| Arc::new(p) as Arc<dyn TextProvider>)
        }
        gemini::ID => {
            GeminiProvider::new(key.expose()).map(|p| Arc::new(p) as Arc<dyn TextProvider>)
        }
        // Not a bug and not a broken key: ComfyUI needs no credential at all,
        // and the image and mesh backends are not wired into this shell yet. A
        // key for one of those can still be stored — it is per installation and
        // will be waiting — there is simply nothing here to ask.
        _ => {
            return Err(WobuError::new(
                Code::Invalid,
                "This build has no way to check that provider's key.",
            )
            .with_detail(id.to_owned()));
        }
    };
    built.map_err(|e| {
        WobuError::new(Code::ProviderUnavailable, "This provider could not be started.")
            .with_detail(e.to_string())
    })
}

/* ── jobs ─────────────────────────────────────────────────────────────────── */
