# Recording and prepared media

Review → Recording exports provider-neutral recording/TTS manifests and imports finished audio
and optional timing. Wobu auditions prepared files; it does not synthesize speech or align it.
Source wording must be approved, current and locked. A translated take additionally requires the
exact current approved translation. Changes to source protection, wording, selected translation,
pronunciation or delivery notes retain earlier takes and make them out of date.

## Handoff and import

Choose a locale and export UTF-8 JSON or RFC4180 CSV. Rows identify the stable string ID, locale,
plural form, speaker, source revision and context guard, exact translation revision, recording notes,
and portable expected audio path. Source metadata and notes are frozen; edit notes in Wobu and
export again. The CSV `frozen_source` cell carries the complete versioned row. Source columns must
agree with it. Import limits are 16 MiB and 100,000 rows.

Place recordings under the chosen external directory at each `audio_path`. Optionally set
`timing_path`, `audio_hash` and `timing_hash`; declared hashes must agree with actual bytes. Empty
hash cells request calculation, never trust an extension. Preview reports unknown/duplicate IDs,
wrong locales/forms, stale source or translation, changed media guards, unsafe paths and invalid
files. Apply repeats validation and imports only eligible rows; missing rows keep existing media.
All duplicate copies are skipped. Partial successes and write conflicts are displayed separately.

A recording binds `(string ID, locale, plural form)` to source and translation revisions. Parameters
are explicit plain-text values keyed by the full placeholder token (including a format suffix).
An unresolved template is refused. A take of `{name}` with `name=Ada` is only valid for Ada;
another parameter value returns no matching clip. Dynamic templates therefore require explicit
text-only fallback for Release even when a frozen take exists. No automatic formatting, arbitrary
region voice substitution or runtime provider call occurs.

## Media and timing subset

Audio is RIFF little-endian PCM WAV, 16-bit mono/stereo, 8–96 kHz, 1 ms–10 minutes, at most 32 MiB.
The actual RIFF size, format, byte rate, frame alignment and sample count are validated. Duplicate
format/data chunks, truncated chunks, compressed/extensible formats and empty audio are refused.
Unicode remains in manifests and spoken text; portable file paths use ASCII without device names,
traversal, absolute roots or symlink ancestors.

Timing is JSON version 1, at most 2 MiB and 10,000 cues:

```json
{"version":1,"audio_hash":"64 lowercase BLAKE3 hex characters","duration_ms":2902,
 "cues":[{"start_ms":100,"end_ms":900,"kind":"viseme","value":"aa"}]}
```

Kinds are `word`, `phoneme`, `viseme`. Intervals are half-open `[start_ms,end_ms)`, ordered and
non-overlapping within each kind, and contained in the exact audio duration. Word/phoneme labels
are nonempty UTF-8 strings (at most 256 bytes). Neutral visemes are `sil PP FF TH DD kk CH SS nn RR
aa E ih oh ou`. An adapter must explicitly map these names to its own blend shapes and convert
milliseconds to its playback clock; no engine mapping is implied. Duration comes from PCM sample
frames, with milliseconds rounded down. A sidecar binds the exact audio hash and duration.

## Canonical history, portability and release

Audio/timing bytes are immutable BLAKE3-addressed files in `assets/media/`. Guarded Production
records retain append-only take history and immutable decision receipts. Publication stages blobs
before linking the new take. Interrupted work can leave unreferenced blobs, but cannot create a
valid link to absent bytes; a conflicting canonical winner is never overwritten. Copying the project
or rebuilding SQLite retains these links. Peer sync advertises historical blob descriptors and safe
file metadata; the existing transfer boundary verifies actual hashes before accepting content.

Recording lists and sync inventory read metadata rather than all audio bytes. Lists explicitly say
that content hashes are verified on audition/export. Missing files and changed sizes are visible;
a same-size corrupt file is refused when auditioned or exported. Audition loads only the requested
take and retains its historical/current label, audio controls, playback milliseconds and active cues.
Note drafts survive paging/dialog navigation and keep their original policy guard for conflict checks.

The recording policy chooses required locales, whether each permits text-only fallback, and which
also require timing. Translated recording locales must be configured in Localisation. Release
refuses missing/outdated required audio or timing unless fallback is explicitly allowed. Development
permits text-only playback. Export verifies source, locale and media observations, hashes and a
120 MiB aggregate media budget before package publication; existing package limits remain 32 MiB
per file and 128 MiB total.

Native packages declare `prepared_media: 1`, keep typed bindings in `media.json`, and list every
blob's hash/size. Empty legacy packages keep their existing bytes/capabilities. `Package::media()`
returns the prepared bundle; `Bundle::lookup(key, parameters)` selects only an exact take;
`Package::media_audio` and `media_timing` expose verified bytes/cues for a host using the existing
runtime line IDs. Keep text visible when a permitted fallback has no clip. Runtime contains no
filesystem, synthesizer or audio-device dependency.

## Verification and limits

The [offline original voiced fixture](../examples/recording-handoff/README.md) demonstrates the
native import/audition/timing workflow. Its cues are hand-authored, not a claim of forced alignment.
Focused tests cover malformed WAV/timing, Unicode CSV roundtrips, unsafe/symlink paths, duplicate
and unknown rows, stale/concurrent imports, unchanged-ID edits, translation decisions, immutable
history, interrupted publication, tampering, reopen/index rebuild and package lookup/release gates.
UI tests exercise guarded partial import, audition/timing updates, error/read-only states, bounded
paging, retained note guards and project-session changes.

N5 Unity/Godot/Unreal adapters and cross-language conformance are excluded. The native Rust lookup
and Wobu audition are the implemented playback surfaces; engine adapter samples remain unverified.
