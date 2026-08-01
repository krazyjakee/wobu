# 08 — Providers & BYOK

Wobu ships with no inference of its own and no Wobu-operated proxy. Every provider is
**bring-your-own-key**: the user pastes credentials they obtained themselves, and Wobu talks
to that provider directly on their behalf.

> ⚠️ **Verify before implementing.** The provider details below were researched on
> **2026-07-31** against vendor documentation. Where a fact could not be confirmed it is
> marked 🚩 rather than guessed. Confirm every endpoint and model ID against a live call
> before writing the adapter — this file is a design brief, not an API reference.

## Three capabilities, not one list

Providers are selected per *capability*, not globally. A user can enhance with Gemini,
generate images on a local ComfyUI, and produce meshes via Hunyuan3D — that combination is
normal, not exotic.

| Capability | Used by | Providers |
| --- | --- | --- |
| **Text** | Enhance | Anthropic, Google Gemini, local (Ollama) |
| **Image** | Generate | ComfyUI (local), Google Gemini, Replicate/fal |
| **Mesh** | Concept 3D | Tencent Hunyuan3D, ComfyUI (local Hunyuan3D weights) |

## Key storage

**Keys are never written into the project folder.** Project folders are designed to be put on
a shared drive ([07](07-file-shares.md)), so a key in `project.json` is a key leaked to
everyone with share access — and to git history, and to whoever the folder gets zipped to.

- Keys live in the **OS keychain** via the `keyring` crate — Secret Service on Linux, Keychain
  on macOS, Credential Manager on Windows — under `wobu/<provider>`.
- Keys are **per-installation, not per-project**. Opening a shared project uses *your* keys.
- `project.json` stores only the *selection*: provider id, model id, and default params. It is
  safe to share, and a collaborator without a key sees "Gemini selected — no key on this
  machine", not an error.
- Keys are held in the Rust process only. They are never sent to the webview, never logged,
  and redacted from error strings before they reach the UI.

Opening a project whose selected provider has no key on this machine drops that capability
into a disabled state with a direct "add key" affordance, rather than failing at generate time.

**Development-time exception.** For local work, `wobu-llm`/`wobu-imagine` may fall back to
environment variables (a repo-root `.env`, e.g. `TencentSecretId` / `TencentSecretKey`) when
the keychain has no entry. Resolution order is **keychain → environment → unconfigured**, and
the env path is compiled out of release builds. `.env` is gitignored; `.env.example` documents
the variable names with empty values. This fallback must never read from inside a *project*
folder — that is the shared-folder leak the keychain rule exists to prevent.

## All network calls happen in Rust

Every provider request is issued from `wobu-llm` / `wobu-imagine` in the Rust process, never
from the webview. Three reasons, in order of importance:

1. The key never enters renderer memory.
2. Streaming, cancellation and retry are already the job queue's problem.
3. **It sidesteps a live CORS bug.** Google's Interactions API client sends an `Api-Revision`
   header, which triggers a CORS preflight that `generativelanguage.googleapis.com` rejects —
   so calling it from a Tauri/Electron webview fails with an opaque `TypeError: Failed to
   fetch` ([js-genai#1723](https://github.com/googleapis/js-genai/issues/1723)). From a native
   HTTP client there is no preflight and no problem.

---

## Anthropic (text)

Base URL `https://api.anthropic.com`, auth header `x-api-key: <key>`, plus a required
`anthropic-version`. Keys come from the Claude Console.

### ✅ Verified against live documentation (2026-07-31)

From [models overview](https://platform.claude.com/docs/en/about-claude/models/overview) and
[pricing](https://platform.claude.com/docs/en/about-claude/pricing):

| Claude API ID | in / out per MTok | context | max output |
| --- | --- | --- | --- |
| `claude-fable-5` | $10 / $50 | 1M | 128k |
| `claude-opus-5` | $5 / $25 | 1M | 128k |
| `claude-sonnet-5` | $3 / $15 | 1M | 128k |
| `claude-haiku-4-5-20251001` | $1 / $5 | 200k | 64k |

Sonnet 5 is $2 / $10 under introductory pricing until 2026-08-31.

**`claude-opus-4-8` and `claude-sonnet-4-6` are legacy**, not current — they are what a model
trained before the Claude 5 generation will offer you, and they are one generation stale. This
is exactly why the id is checked against live docs rather than remembered: a hardcoded id from
training data is a request that fails on the user's machine, and it fails for a reason that
reads like a bug in Wobu.

Wobu defaults to `claude-sonnet-5`. Enhance is a few hundred output tokens of visual
invention: Haiku is a fifth of the price but a generation older, and Opus is five times the
cost for a paragraph about what a censer looks like. Nothing validates the id against a list —
`project.json` carries it — so a model released next month works without a release of ours.

### Request shape

Structured output is **tool use**, not a prompt asking for JSON: one tool whose `input_schema`
is `wobu_core::description_schema(kind)` verbatim, with `tool_choice` pinning it and
`disable_parallel_tool_use`. Streaming deltas arrive as `input_json_delta` — fragments of the
JSON document, which is what the provider trait's deltas are defined to be.

`strict: true` is deliberately **not** set. It would move schema enforcement server-side, but
our shared schema puts a `pattern` on palette entries and string constraints are documented as
unsupported under strict — turning it on trades a rare bad palette entry for every request
failing. Client-side validation catches the same thing for nothing.

**Sampling parameters are not sent at all.** `temperature`, `top_p` and `top_k` were removed
on 4.7+ and are a 400; `thinking: {"type":"disabled"}` is a 400 on Fable 5. Omitting all of
them is the only request shape every current model accepts.

---

## Google Gemini (text + image)

Base URL `https://generativelanguage.googleapis.com`, auth header
`x-goog-api-key: <key>`. Keys come from AI Studio.

**Two APIs currently exist.** The **Interactions API** (`POST /v1beta/interactions`, GA) is
Google's forward path; the legacy `POST /v1beta/models/{model}:generateContent` remains fully
supported. We target the Interactions API, because the usual reason to prefer legacy — the
CORS bug above — doesn't apply to a native HTTP client.

**Path: `/v1beta`.** Re-checked 2026-07-31 while building the adapter (#35). Four of Google's
pages write `/v1beta/interactions` — [structured
output](https://ai.google.dev/gemini-api/docs/structured-output), the [breaking-changes
guide](https://ai.google.dev/gemini-api/docs/interactions-breaking-changes-may-2026) in six
separate curl examples, [streaming](https://ai.google.dev/gemini-api/docs/interactions/streaming),
and the [API reference](https://ai.google.dev/api/interactions-api). Only
[migrate-to-interactions](https://ai.google.dev/gemini-api/docs/migrate-to-interactions) writes
`/v1beta2`, and that same page is the only one still showing the pre-May request shape, so it
reads as stale. 🚩 Still unconfirmed by a live call — nobody on the project has a key. The
adapter sends `/v1beta` and `wobu-llm::gemini` says in a comment that a 404 with nothing billed
means try `/v1beta2`.

**Pin the revision — but it no longer does anything.** The API is versioned by date via an
`Api-Revision` header. The current shape is `2026-05-20`: default since 2026-05-26, and the
revision before it was *removed* on 2026-06-08. That revision is what renamed `outputs` to
`steps` and folded `response_mime_type` into `response_format`, so anything written against an
older example is a 400. Both adapters send the header explicitly.

Corrected 2026-08-01 (#52): the [breaking-changes
guide](https://ai.google.dev/gemini-api/docs/interactions-breaking-changes-may-2026)'s own
timeline says of 2026-06-08 — "Legacy schema removed for Interactions API. **`Api-Revision`
header ignored.**" Neither the image-generation guide nor the API reference mentions the header
at all. So sending it is a no-op, kept because it records in the request which shape the
adapters were written against. Do not rely on it to hold a shape.

### Text — Enhance

Default model **`gemini-3.6-flash`**. Streaming is `"stream": true` in the body, returned as
SSE — no separate endpoint, no `?alt=sse`.

Verified 2026-07-31 against the [model
page](https://ai.google.dev/gemini-api/docs/models/gemini-3.6-flash) and
[pricing](https://ai.google.dev/gemini-api/docs/pricing):

| Model | in / out per Mtok | cached in | free tier |
| --- | --- | --- | --- |
| `gemini-3.6-flash` | $1.50 / $7.50 | $0.15 | yes — "Free of charge" |
| `gemini-3.5-flash` | $1.50 / $9.00 | $0.15 | yes — "Free of charge" |
| `gemini-3.5-flash-lite` | $0.30 / $2.50 | $0.03 | — |
| `gemini-3.1-pro-preview` | $2 / $12 to 200k, $4 / $18 beyond | $0.20 | — |

`gemini-3.6-flash` is 1,048,576 in / 65,536 out.

🚩 Still do not assert a *quality* ranking between `gemini-3.6-flash` and `gemini-3.5-flash`;
Google's own model page describes the older one as more capable. The adapter defaults to 3.6
on price alone (same input, cheaper output) and says so. Benchmark on our actual Enhance prompt
before writing UI copy that claims more than that.

Structured descriptions use a top-level `response_format` (note: **not** inside
`generation_config`). The whole request, as the adapter sends it:

```json
{
  "model": "gemini-3.6-flash",
  "input": "...",
  "stream": true,
  "store": false,
  "system_instruction": "...",
  "generation_config": { "max_output_tokens": 4096 },
  "response_format": {
    "type": "text",
    "mime_type": "application/json",
    "schema": { "type": "object", "properties": { }, "required": [] }
  }
}
```

`system_instruction` is a **plain string at the top level** — not the legacy
`{parts: [...]}` object. `max_output_tokens` is the one setting that lives under
`generation_config`. **`store` defaults to true**: the API keeps the request and the response
server-side so a later call can chain onto them. Enhance never chains, and the payload is
somebody's unpublished world, so the adapter opts out.

**Streaming shape.** Events are `interaction.created`, `interaction.status_update`,
`step.start`, `step.delta`, `step.stop`, `interaction.completed`, `error`, then a
`data: [DONE]` sentinel. Each payload repeats its own name in `event_type`. Two things a naive
reader gets wrong:

- **Steps are numbered and thinking is one of them.** A `thought` step emits `step.delta`
  events on the same event type as the answer, carrying `thought_signature` and — with
  `thinking_summaries` on — prose. Track the `index` of the `step.start` whose
  `step.type == "model_output"` and drop everything else, or the model's reasoning ends up
  concatenated in front of the JSON.
- **There is no finish-reason field.** Hitting `max_output_tokens` shows up as the
  interaction's `status` being `incomplete` ("completed, but contains incomplete results").
  The other terminal statuses are `completed`, `failed`, `cancelled`, `budget_exceeded`.

**Usage** is `usage.total_input_tokens` / `total_output_tokens` / `total_cached_tokens` /
`total_thought_tokens` / `total_tokens`, on `interaction.completed` — and also as
`metadata.total_usage`, which the docs say may accompany *any* streamed event. Read both, or a
cancelled call reports zero. 🚩 Two readings the docs don't settle and a live call would:
whether `total_cached_tokens` is a subset of `total_input_tokens` (the wording "the cached part
of the prompt" says yes, and the adapter subtracts), and whether `total_output_tokens` already
includes thoughts (the adapter adds them, which errs high rather than low).

**Errors** have their own documented table
([api-errors](https://ai.google.dev/gemini-api/docs/api-errors)): a machine-readable snake_case
`code` plus `message`, delivered as an HTTP body or — for a failure after the 200 — as an
`error` stream event with the same fields. Codes: `invalid_request`, `parameter_unknown`,
`authentication`, `permission_denied`, `not_found`, `model_not_found`, `rate_limit_exceeded`,
`quota_exceeded`, `cancelled`, `api_error`, `service_unavailable`, plus a separate set for
blocked generations (`safety`, `recitation`, `prohibited_content`, `spii`, `blocklist`, …).
Google's edge still emits the older `google.rpc.Status` envelope for some refusals — integer
`code`, name in `status` (`RESOURCE_EXHAUSTED`, `PERMISSION_DENIED`, `FAILED_PRECONDITION`) —
so handle both. `FAILED_PRECONDITION` is specifically "the free tier is not available in your
country and billing is not enabled", which is a *working* key: don't report it as a bad one.
A 429 carries the wait in the body as a `google.rpc.RetryInfo` detail (`"retryDelay": "42s"`),
not in a `Retry-After` header.

The accepted schema is a **subset** of JSON Schema. Documented as supported, verbatim from the
structured-output page on 2026-07-31: `string` `number` `integer` `boolean` `object` `array`
`null`; `title` `description`; `properties` `required` `additionalProperties`; `enum` `format`;
`minimum` `maximum`; `items` `prefixItems` `minItems` `maxItems`; `anyOf`; `$ref`. Google states
plainly that "not all JSON Schema features are supported" and that deeply nested schemas may be
rejected.

🚩 **`pattern` does not appear on that page at all** — neither as supported nor as unsupported —
and the page does not say what happens to a keyword it doesn't know. Our shared description
schema puts a `pattern` on palette entries, and the adapter sends the schema **unedited**, the
same bytes Anthropic gets, because a per-vendor edit is exactly the drift `wobu-core::schema`
exists to prevent. If a live call comes back 400 with `invalid_request` naming `pattern`, that
is a `wobu-core::schema` change (drop the `pattern`, keep `is_hex_color` as the enforcement
point) and not an adapter workaround. If it comes back 200, the client-side validator catches a
bad palette entry either way.
🚩 `oneOf`, `propertyOrdering` and `responseJsonSchema` are still absent from the current docs.
Don't use them.

> **`temperature`, `top_p` and `top_k` were deprecated on 2026-07-21.** Do not build sampling
> sliders into Settings. 🚩 The deprecation date was not re-confirmed in July 2026; the model
> and pricing pages don't mention sampling at all. The adapter sends none of them regardless,
> which is the safe shape either way.

### Image — Generate

| Model | Notes |
| --- | --- |
| `gemini-3.1-flash-image` | default; up to 4K |
| `gemini-3-pro-image` | highest quality; largest reference budget |
| `gemini-3.1-flash-lite-image` | cheapest, **1K only** |

`imagen` is deprecated; don't add it.

Images return as **inline base64** — there are no URLs — and all output carries **SynthID**
watermarking, which is worth stating in the UI since this is concept art headed for a
pipeline. Reference images are passed the same way (base64 inline, or via the Files API for
larger payloads), which maps cleanly onto our AssetLink roles.

**Reference budgets, per model** — these feed the image budget in [04](04-influence-engine.md):

| Model | objects | characters | style refs |
| --- | --- | --- | --- |
| `gemini-3.1-flash-lite-image` | 14 | – | – |
| `gemini-3.1-flash-image` | 10 | 4 | – |
| `gemini-3-pro-image` | 6 | 5 | 3 |

Aspect ratios: use the intersection both Google docs agree on — `1:1 3:2 2:3 3:4 4:3 4:5 5:4
9:16 16:9 21:9`. Sizes are `512px`, `1K`, `2K`, `4K`, uppercase K required.

Re-checked 2026-08-01 (#52) and still the right list. The [API
reference](https://ai.google.dev/api/interactions-api)'s `aspect_ratio` enum has **fourteen**
values — those ten plus the extreme panoramas `1:4 4:1 1:8 8:1` — and the guide's table for
`gemini-3-pro-image` lists exactly the ten above. The intersection is what we offer.

### The image config field: settled, 2026-08-01

The 🚩 below was **resolved by re-reading the live docs while building #52**. The pages no
longer disagree, and neither candidate the doc recorded was quite right.

The current shape is a **top-level `response_format` whose `type` is `"image"`** — not a nested
`response_format.image`, and not `generationConfig.imageConfig`. Verbatim from
[image-generation](https://ai.google.dev/gemini-api/docs/image-generation) (page last updated
2026-07-30):

```json
{
  "model": "gemini-3.1-flash-image",
  "input": [
    { "type": "text", "text": "..." },
    { "type": "image", "mime_type": "image/png", "data": "<BASE64>" }
  ],
  "response_format": { "type": "image", "aspect_ratio": "16:9", "image_size": "2K" }
}
```

The [API reference](https://ai.google.dev/api/interactions-api) (last updated 2026-07-31) names
this variant `ImageResponseFormat`: `aspect_ratio`, `image_size` (`512` / `1K` / `2K` / `4K`),
`mime_type`, and `delivery` (`inline` / `uri`). The
[breaking-changes](https://ai.google.dev/gemini-api/docs/interactions-breaking-changes-may-2026)
page states it outright — "`image_config` moves from `generation_config` to `response_format`"
and "the new schema removes `image_config` from `generation_config`" — and the legacy schema was
removed on 2026-06-08. `imageConfig` in camelCase appears on none of the five pages checked; the
disagreement the doc recorded was almost certainly the "Before (legacy)" half of that migration
example.

**Input and reference images** are one flat array of typed blocks — `{type: "text", text}` and
`{type: "image", mime_type, data}`, base64 inline. There is **no `role` and no `parts`**; those
strings appear nowhere in the reference. **Output** is the same block shape, on the `content` of
the `model_output` step: `{type: "image", data: "<base64>", mime_type}`. Streaming carries it as
an `ImageDelta` on `step.delta`.

Consequence for [04](04-influence-engine.md) and #86: **the reference buckets have no
corresponding request field.** Objects / characters / style refs are how Google *counts*
references — the 14, 10+4, 6+5+3 table above is real and documented — but the request itself is
an undifferentiated list. The buckets are a budget, not a routing table, and reading order is
the only signal about a reference that survives.

Still 🚩:

- **The top-level envelope of a non-streaming response.** Every published example is SDK code
  reading `interaction.steps`, and the reference notes `output_image` is "added by the SDK". So
  whether the raw body *is* the interaction or wraps it under `interaction` is unconfirmed. The
  adapter accepts both.
- **The sub-1K token.** The reference enum says `512`; the guide's prose says `512px`. Unresolved,
  and unreached — no ceiling the adapter declares produces a long side under 513, so it is never
  sent. Also flash-only: pro does 1K/2K/4K, lite is 1K only.
- **Output `mime_type`.** `ImageResponseFormat.mime_type` lists only `image/jpeg`, while every
  input example uses `image/png`. The adapter omits the field and reads what arrives.
- There remain multiple credible reports of `imageSize` and `aspect_ratio` being **silently
  ignored**. This is a design instruction regardless: **read back the actual returned dimensions
  and trust those, not the request.** The adapter does, off the image header, sharing
  `wobu-imagine`'s `dimensions.rs` with the ComfyUI one.

**SynthID** is confirmed, verbatim and twice on the image-generation page: "All generated images
include a SynthID watermark." The adapter declares it unconditionally as data on the outcome
(`GeneratedImage::watermark`) rather than as a log line, for #54's card and #59's export.

**Image-specific error codes**, from [api-errors](https://ai.google.dev/gemini-api/docs/api-errors)
on 2026-08-01, on top of the shared table above: `image_safety`, `image_prohibited_content`,
`image_recitation`, `image_other` are blocked generations, and `no_image` is "the model was
unable to generate an image" — which is not a refusal and needs its own answer, because a stated
refusal is a prompt to edit and a silent empty result is a bug report.

### The thing that will generate support tickets

**Gemini image generation has no free tier.** `gemini-3.6-flash` text is free; every image
model is billing-only. A user who pastes a working key, successfully enhances a species, and
then gets an error on Generate will reasonably conclude Wobu is broken.

✅ Confirmed 2026-08-01 against [pricing](https://ai.google.dev/gemini-api/docs/pricing) (page
last updated 2026-07-30): all three image models show Free Tier **"Not available"** for input
and output, on both Standard and Batch. Per-image output cost at the sizes we send:

| Model | 1K | 2K | 4K |
| --- | --- | --- | --- |
| `gemini-3.1-flash-lite-image` | $0.034 | – | – |
| `gemini-3.1-flash-image` | $0.067 | $0.101 | $0.151 |
| `gemini-3-pro-image` | $0.134 | $0.134 | $0.240 |

So the adapter must detect this specific failure and say *"Gemini image generation requires
billing enabled on your Google account"* with a link — not surface a raw 429/403. Ideally we
probe capability at key-entry time and show it in Settings before the user ever hits Generate.

**How the failure actually arrives** (#52, 2026-08-01). `FAILED_PRECONDITION` appears on *none*
of the current pages — not image-generation, pricing, billing, rate-limits, troubleshooting or
api-errors — and the Interactions API has moved to a flat `{"error": {"code": "<snake_case>",
"message": "..."}}` envelope with no `google.rpc` status details. What an account without billing
gets on an image model is therefore an ordinary **429 `quota_exceeded` / `rate_limit_exceeded`
whose message names the free-tier metric and `limit: 0`**. That is not a wait: waiting out a
limit of zero is waiting forever, and a queue that retries it hammers the key. The adapter
matches on the message, maps it to `BillingRequired` rather than `RateLimited`, and keeps
`FAILED_PRECONDITION` mapped too because the edge still speaks the old envelope. 🚩 The `limit: 0`
message text is from community reports rather than from a Google page, and is the one part of
this path that a live call should confirm.

🚩 Concrete free-tier RPM/TPM/RPD numbers are deliberately unpublished and vary; do not
hardcode them. Read limits from error responses and back off. Free-tier availability in the
EEA/UK/CH could not be confirmed.

Re-checked 2026-08-01: [rate-limits](https://ai.google.dev/gemini-api/docs/rate-limits) now
publishes *no* per-model RPM/TPM/RPD at all — "rate limits depend on a variety of factors (such
as your usage tier) and can be viewed in Google AI Studio… Specified rate limits are not
guaranteed". Only Batch enqueued-token figures and a rolling ten-minute spend cap are given. It
also defines an image-specific dimension, **IPM** (images per minute). And note that the current
error page documents **no `RetryInfo`** and asks for plain exponential backoff — so a `retryDelay`
is a bonus to read where it appears, never something to require.

---

## Tencent Hunyuan3D (mesh)

Hunyuan3D 3.1 is our concept-3D backend. It is **not** shaped like the other providers, and
the differences are load-bearing — read this before estimating the work.

### Pick the right namespace

Tencent ships this product under three overlapping API namespaces, and choosing wrong is the
most likely way to lose a day:

| | Host | Service | Version | Has 3.1? |
| --- | --- | --- | --- | --- |
| **International** ✅ | `hunyuan.intl.tencentcloudapi.com` | `hunyuan` | `2023-09-01` | **yes** |
| International `ai3d` ⛔ | `ai3d.intl.tencentcloudapi.com` | `ai3d` | `2025-05-13` | endpoint not live |
| Mainland China | `ai3d.tencentcloudapi.com` | `ai3d` | `2025-05-13` | yes |

Target the first. Note the counter-intuitive part: the *newer* capability lives under the
*older-looking* version string. The mainland namespace has a richer action set (auto-rigging,
motion, retopology) that international does not, and international Pro additionally lacks the
`ResultFormat` parameter — so we take the default format set there.

`ai3d.intl` is a dead end regardless of what the SDK source suggests: a signed
`QueryHunyuanTo3DProJob` against it returns `ResourceUnavailable.InterfaceNotExist`.

### ✅ Verified against a live account (2026-07-31)

A signed request from a **non-mainland account** succeeded — the credentials probe returned
`FailedOperation.JobNotFound` for a bogus `JobId`, which is the request completing normally.
So: non-mainland signup and service activation work, and our TC3-HMAC-SHA256 implementation is
correct. The earlier viability blocker is closed.

**`Region` is far more restrictive than the general Tencent Cloud region list.** Sweeping
twelve regions against `QueryHunyuanTo3DProJob`, exactly three are supported — everything else
returns `UnsupportedRegion`:

| Region | |
| --- | --- |
| `ap-singapore` | Asia-Pacific |
| `na-siliconvalley` | North America |
| `eu-frankfurt` | Europe |

Rejected: `ap-guangzhou`, `ap-hongkong`, `ap-tokyo`, `ap-seoul`, `ap-bangkok`, `ap-mumbai`,
`ap-jakarta`, `na-ashburn`, `na-toronto`, `sa-saopaulo`.

This matters more than it looks. `ap-guangzhou` appears throughout Tencent's own documentation
examples and is the obvious default to reach for — and it does not work on the international
endpoint. The adapter should offer only these three, default by rough geographic proximity,
and remember that **the poll must target the same region as the submit**.

### 3.1 is a parameter, not an endpoint

You call `SubmitHunyuanTo3DProJob` with `Model: "3.1"` (default is `3.0`). It is Pro-only —
the Rapid action has no `Model` field.

**The lucky finding: 3.1's whole headline feature is multi-view input, and we are a multi-view
generator.** 3.0 accepts front + 3 views; 3.1 accepts front + 7:

```
left · right · back · top · bottom · left_front · right_front
```

Our Turnaround preset already exists to produce consistent multi-view sheets. It should be
**redefined to emit exactly these seven view types plus front**, so its output drops straight
into `MultiViewImages` with no intermediate step. That is a much stronger 3D pipeline than
single-image-to-3D, and it falls out of the influence engine for free. See
[04](04-influence-engine.md).

**The cost of 3.1:** `LowPoly` and `Sketch` generate modes are unavailable. Since `Sketch` was
the *only* mode permitting text and image together, **3.1 has no text+image conditioning path
at all** — the compiled prompt cannot ride along with the images. Text-to-3D and image-to-3D
are mutually exclusive. For us that is acceptable: by the time we reach 3D, all the influence
has already been baked into the turnaround images.

### Job model

Asynchronous submit-then-poll, which fits the existing job queue:

1. `POST /` to the host with `X-TC-Action: SubmitHunyuanTo3DProJob`, `X-TC-Version: 2023-09-01`,
   `X-TC-Region: <region>`, params in the JSON body → returns `JobId`.
2. Poll `QueryHunyuanTo3DProJob` with that `JobId`. `Status` is exactly
   `WAIT | RUN | FAIL | DONE`.
3. `DONE` yields `ResultFile3Ds[]` of `{ Type, Url, PreviewImageUrl }`.

Two expiry traps: **`JobId` is valid 24 hours** and **result `Url`s are valid 24 hours**. Never
persist a result URL — download into `assets/meshes/` immediately on `DONE`.

Also: the OBJ `Url` is a **`.zip`** (mesh + `.mtl` + texture maps), not a bare mesh, so the
downloader must unzip. And the international docs list `Type` values that contradict GLB being
returned — treat `Type` as an open string and switch on it defensively rather than as an enum.

Key parameters: `EnablePBR` (default false), `FaceCount` (default 500000, range 3000–1500000),
`GenerateType` (`Normal` / `Geometry`; `LowPoly` and `Sketch` unavailable on 3.1).

Limits: **3 concurrent Pro jobs**, 20 requests/second. The queue must respect the concurrency
cap or we'll generate our own rate-limit errors.

### Input constraints

These bound what the Turnaround preset is allowed to emit, so they live in its
`image_constraints` and are enforced again by the validated `Turnaround` batch constructor and
the wire adapter rather than being discovered at submit time:

| | |
| --- | --- |
| Formats | `jpg png jpeg webp` single-image; **`JPG`/`PNG` only** for multi-view |
| Resolution | min side ≥ 128, max side ≤ 5000 |
| `ImageUrl` | ≤ 8 MB |
| `ImageBase64` | ≤ 6 MB pre-encode (base64 inflates ~30%) |
| Multi-view total | all images combined ≤ 6 MB pre-encode |
| Views | exactly one image per view type, no duplicates |

Tencent's own input guidance — plain background, no text, single object, subject filling >50%
of frame — is precisely what our Turnaround preset should be tuned to produce anyway.

`Turnaround::new` accepts only the eight `View::ALL` tags in preset order, one each. It reads the
real MIME and dimensions from each header rather than trusting metadata, rejects a mismatched
label, and sums the unencoded bytes across the batch. `MeshRequest::from_turnaround` takes that
validated value, so the named preset path cannot construct a request the 3D stage would reject;
the lower-level single/partial-view path remains available for Hunyuan's ordinary image-to-mesh
mode and receives the same per-image wire checks.

### BYOK is genuinely harder here, and users must be told

This is the part that differs most from Gemini, and it needs to be designed for rather than
discovered:

- **There is no bearer API key.** Authentication is a `SecretId` + `SecretKey` pair signed with
  **TC3-HMAC-SHA256**, an AWS-SigV4-style canonical-request construction. We must implement
  signing in `wobu-imagine`; it is neither optional nor trivial. Budget for it. (A working
  reference implementation was verified against a live account — roughly 40 lines. The signed
  header set is `content-type;host;x-tc-action`, with `X-TC-Timestamp`, `X-TC-Version` and
  `X-TC-Region` sent unsigned alongside `Authorization`.)
- **The `SecretKey` is an account-wide master credential**, not a scoped token — materially
  more dangerous to hold than an OpenAI-style key. Keychain storage is mandatory, and the
  onboarding copy should actively steer users to create a **CAM sub-account key scoped to the
  3D service** rather than pasting their root credentials.
  🚩 The exact CAM policy/action prefix to recommend is unverified.
- **`Region` is a required parameter**, restricted to the three regions verified above, and the
  poll must target the same region as the submit.
- **Signatures expire after 5 minutes of clock skew** (`AuthFailure.SignatureExpire`). Desktop
  clocks drift, so this error must map to a specific "check your system clock" message.
- **A fresh account hits `FailedOperation.ServiceNotActivated` before anything works** — the 3D
  service requires explicit activation in the console. This should be an onboarding step with
  a link, not a runtime error.

### Remaining unknowns

Viability and regions are settled. Still open, none of them blocking:

- 🚩 The CAM policy / action prefix to recommend for a least-privilege sub-account key.
- 🚩 Current pricing and free-credit allowance — the international `Query` response omits the
  `ResultCreditConsumed` field that the mainland one returns, so **we cannot read spend back
  from the API** and the cost estimate will have to be a local model of published prices.
- 🚩 Whether `3d.hunyuanglobal.com` issues credentials of its own. Evidence points to the
  Tencent Cloud console being the only source, which matches the `SecretId`/`SecretKey` shape
  we verified.

### Local fallback is a different tier, not a fallback

Tencent's open-weight releases stop at **Hunyuan3D-2.1**; there are no 3.x weights, and the
strong indication is that 3.x is cloud-only. So running Hunyuan3D locally under ComfyUI gets
you a materially older and lower-quality model — worth offering for cost and privacy reasons,
but it must be presented as a **different quality tier**, not a drop-in substitute when the
cloud key is missing.
🚩 ComfyUI node availability and VRAM requirements for 2.1 are unverified.

---

## Capability negotiation

Each adapter declares what it can do, and the UI adapts rather than failing late:

```rust
pub struct Capabilities {
    pub max_resolution: Resolution,
    pub aspect_ratios: Vec<AspectRatio>,
    pub image_refs: ImageBudget,            // provider counting axis
    pub reference_mechanisms: ReferenceMechanisms, // adapter routing axis
    pub loras: bool,
    pub negative_prompt: bool,              // see below — Gemini image has none
    pub requires_billing: bool,
    pub streaming_preview: bool,
}
```

**Declared per model, not per backend** (#50). One `Capabilities` for "Gemini" would have to
be the worst of the three image models — `gemini-3.1-flash-lite-image` is 1K only and counts
fourteen undifferentiated references, `gemini-3-pro-image` goes to 4K and counts six, five and
three — which would hide two thirds of the reference budget the user is paying for. So
`capabilities()` takes the model id, and a model id nothing in the registry names still gets an
answer: the adapter's most conservative *registered* budget, never `ImageBudget::unlimited`.

`negative_prompt` is not in the original sketch and was added because without it the
negotiation is not total. `never:` is the one section every kind is required to declare, it
compiles to `FragmentTarget::Negative`, and the Gemini image API has no field to put it in —
Imagen's `negativePrompt` went with Imagen. ComfyUI has one. Without the flag the only possible
behaviour is to drop user-authored canon in silence; with it, the user is told the `never:`
list is not enforced on this backend. It is *not* folded into the positive prompt as a
"without X" clause, which reads to a text encoder as a request for X.

`image_refs` is deliberately **not** a map keyed by our own `AssetRole`. The caps are declared
in the provider's counting vocabulary — objects, characters, style refs — and `wobu-influence` owns the
mapping from our seven roles onto those three buckets (`capability.rs`, #44). Keying the
capability by `AssetRole` would push that judgement into every adapter and let two of them
disagree about which bucket a `pose` reference competes in.

The buckets are **partitions of one reference budget, not separate budgets**: every row of the
table above sums to 14. A model that declares no character category is not refusing images of
people — it has one undifferentiated pool, so those references are counted as objects and
*share* the object cap. Treating an undeclared bucket as its own pool would build a 20-image
request for a model that takes 10, and the provider would be the one to point that out, after
payment.

Buckets are counting, not routing. Gemini's request contains one undifferentiated list of image
blocks; there is no object field or style-reference field despite the separate documented quota
columns. ComfyUI goes the other way: ControlNet and IPAdapter are graph mechanisms, and their
inputs cut across the provider buckets. A pose (`Characters`) and silhouette (`Objects`) both use
the structure mechanism, while silhouette and palette share `Objects` but need different
mechanisms.

`reference_mechanisms` is therefore a second, independent budget with `image_prompt` and
`structure` pools. Negotiation applies it before `image_refs`. This can express a ControlNet graph
with one structure input and no image-prompt input without lying about ComfyUI having `0/0/0`
provider buckets. A reference rejected by either layer remains attributed in that layer's report:
mechanism-unavailable and mechanism-full are capability downgrades; a provider quota overflow is
an image-budget drop. Neither is silent.

The chosen `ReferenceMechanism` travels on `ImageRequest::references` for adapter routing. The
`RefBucket` travels beside it only so the Inspector and generation snapshot can say which provider
quota it consumed. Gemini ignores both on the wire because its API accepts only the flat list;
the original `AssetRole` is still retained in the request for attribution. Explaining those roles
to a model whose wire format has no labels is prompt-attribution work, not a reason to overload
quota buckets as fictional request fields.

Consequences the user can actually see:

- A backend with no structure mechanism shows structure references as visibly **downgraded to
  mood-board-only**, rather than silently ignoring them.
- Aspect ratios the backend doesn't support don't appear in the dropdown.
- Per-role reference caps drive the image budget, so the Inspector can say `3/3 style refs`.

## Cost and consent

BYOK means the user pays per call, so the app must never surprise them:

- The Generate button shows an **estimated cost** for the batch when the selected provider is
  paid (Gemini image is roughly $0.05–0.24 per image depending on model and size; treat these
  as indicative and re-check).
- Local ComfyUI shows no cost — that asymmetry is the point, and it's a good default.
- A per-project **spend ceiling** with a hard stop, because a turnaround loop is exactly the
  kind of thing that runs 200 images unattended.
- Every generation record already stores the provider, model and params, so actual spend is
  reconstructable from the project folder.
