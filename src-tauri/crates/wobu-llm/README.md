# Structured authoring requests

`TextProvider::structured(&StructuredRequest, &mut dyn DeltaSink, &Cancel)` supplies a
caller-owned JSON Schema to the existing text adapters. `supports_structured()` defaults to
false, with an unsupported error from the default method, so existing fake/third-party
Enhance providers keep working. Anthropic and Gemini opt in. This capability does not promise
that every selected model accepts every schema keyword.

The request carries model, optional system instruction, prompt, schema and output-token cap.
Anthropic sends one forced `record_narrative_text` tool with the schema as `input_schema` and
parallel tool use disabled. This reuses the existing tool mechanism; it does not enable strict
schema decoding or execute a tool. Gemini sends the existing Interactions API
`response_format: {type: "text", mime_type: "application/json", schema: ...}` and `store: false`.
Existing endpoints, authentication, pinned revisions and Enhance behavior are retained.
These request and streaming shapes were checked against the official
[Anthropic tool guide](https://platform.claude.com/docs/en/agents-and-tools/tool-use/define-tools),
[Gemini structured output guide](https://ai.google.dev/gemini-api/docs/structured-output) and
[Gemini streaming guide](https://ai.google.dev/gemini-api/docs/streaming) on September 8, 2026.
No paid-provider call was performed for this implementation.

A `StructuredOutcome` always carries the latest reported `Usage`, alongside `Result<String>`.
The successful string is the original complete JSON object, including duplicate keys if the
provider emitted them. Syntax is checked without rewriting it. **The compiler must still reject
duplicate keys, wrong/missing slot or speaker IDs, unexpected fields, logic injection, and
invalid lengths before accepting a candidate.** Stream deltas are progress only, never an
accepted candidate. Cancellation aborts the network body and preserves usage already reported;
zero usage means unknown, not proof that the call was free.

The structured path accepts at most 64 KiB of generated JSON and 1 MiB of total SSE wire bytes,
including comments, unknown events and incomplete frames. Exceeding either limit fails closed.
It requires one output block/step and a successful terminal state, rejects malformed events,
and drops a truncated or failed response. UTF-8 corruption is rejected rather than repaired.
The limits apply to structured calls; they do not silently change Enhance's existing limits.

Only an actual rejected HTTP 429 returns `RateLimited` with the retry header. A rate-limit event
inside an accepted stream becomes `Unavailable`, with usage retained: a caller must not infer
that replay is free. Structured requests perform no retries. The existing general-purpose
`Error::is_retryable` is **not** a safe automatic-retry policy for narrative generation; its
caller must apply the stricter batch policy. Structured failures omit provider body messages,
URLs and raw transport errors so they cannot echo credentials into receipts. Non-success HTTP
bodies are dropped without being read.

`tests/structured_http.rs` runs both real HTTP adapters against loopback fixture servers. It
checks actual headers/request bodies, raw output and usage, malformed/truncated/oversized
streams, HTTP 429 versus stream errors, pre-cancel, and socket closure while cancellation races
a quiet response. It is local transport evidence, not external-provider or model-quality evidence.

For the narrative speaker union, Gemini's wire schema replaces `oneOf` between a string enum
and an object with `anyOf`: those alternatives cannot overlap, so the accepted values are
identical. The canonical request schema is unchanged. Overlapping unions, existing `anyOf`
conjunctions and enum/const instance data are never rewritten. The official guide documents
`additionalProperties` and `minItems`/`maxItems`; string `minLength`/`maxLength` are not part of
its documented subset and may not be enforced remotely. They remain in the request and must
be checked by the local compiler. No success from this adapter substitutes for that check.
