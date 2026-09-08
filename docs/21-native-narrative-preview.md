# Native narrative Preview walkthrough

Issue #195 completes explicit save migration, actual runtime decision traces and simulated host
command results. The [runtime contract](17-narrative-runtime-contract.md) defines the APIs and
transaction semantics. This walkthrough validates the desktop author experience in a real Tauri
process with handwritten Ashfall dialogue; it does not use a mocked browser IPC implementation.

## Reproduce the authored project

Run these commands from a checkout, choosing a new project-copy path:

```sh
cp -R examples/Ashfall.wobu /tmp/Ashfall-preview.wobu
cd src-tauri
cargo run -p wobu-narrative-runtime --example ashfall_preview_fixture -- /tmp/Ashfall-preview.wobu
cd ..
npm run tauri dev
```

The fixture writer refuses to overwrite an existing narrative directory. Its checked-in Rust source
contains every handwritten line, stable ID, predicate and effect; no provider or generation call is
involved. Open the project copy in Wobu, then Narrative → **Ashfall council hearing** → Script →
Preview. The three development warnings are expected: these authored variants are drafts, suitable
for structural Preview but not approved release dialogue.

In **Host command signatures**, enter `{"file_record":[]}`. This command has no arguments. Set
starting `trust` to 40, `record_confirmed` to false and `repeat_wait` to false. The latter two variables
belong to the host; `trust` belongs to narrative logic and is bounded to 0–100.

## Walkthrough and evidence

The [native recording](evidence/narrative-195/native-walkthrough.mp4) is 3 minutes 28 seconds long.
It was captured on Linux at 1440×1100 using Wobu's GTK/WebKit Tauri window in an isolated Xvfb display,
a private D-Bus session and private XDG config/data/cache directories. Vite served the frontend on
port 1495; compiled Rust handled every compile, step and restore invocation. Mouse and keyboard
input came from `xdotool`, and FFmpeg recorded the display. This is desktop evidence, not game-engine
adapter or cross-language conformance evidence.

1. Start with `trust = 99`, continue the player line, then choose **Present the logbook**. The +10
   effect exceeds the declared bound. Preview shows the [overflow error](evidence/narrative-195/native-overflow.png)
   and retains the choice and trust 99. Failed execution does not partially commit state.
2. Restart with `trust = 40`. Read the [player line](evidence/narrative-195/native-line.png), continue
   and choose **Present the logbook**. The ordered effects change trust to 50 and yield `file_record`.
3. Save a snapshot while the command is pending. Select **Fail command**, then **Cancel command**.
   Both retain the pending command for an explicit retry; the [failure screenshot](evidence/narrative-195/native-command-failed.png)
   shows the paused command and error. Restore the saved snapshot to return to its pending boundary.
4. Set host output `record_confirmed` to true and acknowledge. The next selected line is
   [“The council accepts the record. An investigation will begin.”](evidence/narrative-195/native-command-success.png)
   Trust remains 50 and the returned host variable becomes true.
5. Use **Open this line in Script**, then inspect the same saved scene in
   [Source](evidence/narrative-195/native-source.png). Return to Preview and continue to **Hearing
   complete**. Expand playback trace to inspect actual predicate results, routes and ordered effects.
   [The final trace screenshot](evidence/narrative-195/native-trace.png) shows trust 40 → 50 and an
   **Open choice in Script** source link. Restore the checkpoint again after End: the
   [pending command returns](evidence/narrative-195/native-restored-command.png), with trust still 50.
6. Open **Ashfall bounded loop check** → Preview, register the same command signatures (the whole
   project is compiled), and set `repeat_wait = true`. Starting Preview returns the
   [1,000-step budget error](evidence/narrative-195/native-loop-limit.png), without freezing the app.
   Set it false and restart to reach [“Loop guard cleared”](evidence/narrative-195/native-loop-cleared.png).

The recording precedes one final trace-size refinement: its effect `after` objects include other
state variables. The final trace screenshot was captured in a rebuilt native binary and confirms
both before and after now include only variables used by that effect. Execution results are unchanged.

The [canonical-file audit](evidence/narrative-195/canonical-check.json) contains SHA-256 values before
and after the walkthrough for all 22 initial canonical files: project metadata, world nodes, edges
and narrative source. Every hash matches. Local indexes/session metadata are excluded from canonical
content. Inputs, host simulation, traces and checkpoint restore do not save changes to story canon.

## Acceptance and automated checks

| Acceptance | Implementation and evidence |
| --- | --- |
| Explicit validated migration | `Runtime::restore_with_migration` validates the original graph/save, expected source and target hashes, migrated state domains, cursor/visits and pending commands. Ordinary restore still rejects incompatible content. `trace_migration.rs` exercises successful migration, invalid callback results, changed command signatures/source IDs, and unchanged command token identity. |
| Actual condition/effect traces | Exported `TraceSite`, `TraceEvent`, `TraceRecord` and `ExecutionTrace` contain stable source IDs, evaluated predicates, actual routes, ordered effect values and command results. Short-circuited predicates are absent; rejected transactions are marked rolled back. At most 2,048 records per action are retained, with an explicit omitted count. Preview retains 100 actions and links their source. |
| Full paused host results | Preview offers typed host outputs, failure and cancellation using the paused compiled schema. Bridge tests cover ownership/type/domain validation, save/restore while pending, identical acknowledgement retry and conflicting result rejection. UI tests cover source edits during a pause, retained failures and native error envelopes. |
| Native Ashfall walkthrough | Recording, screenshots and canonical audit above demonstrate the actual Tauri workflow, including overflow and bounded-loop behavior. |

Focused validation: 20 runtime integration tests, six Preview bridge tests and eight Preview UI tests;
Rust formatting and Clippy for runtime/app targets; frontend type checking, lint and code-health checks.
Native binary builds use the repository's pinned Rust toolchain. The complete integration branch must
also pass the repository-wide gates before merging.

Migration is an opt-in Rust API, not an automatic desktop checkpoint upgrade. Checkpoints remain
pinned to their compiled graph. Trace retention is explicitly bounded and may be incomplete for very
large actions. Seed storage still reserves a future policy; selection remains authored first-match.
Subsequent work adds [persisted scenarios](22-narrative-scenarios.md) and
[native packaging](20-native-narrative-packages.md). Flow overlays (#188) remain planned; N5 engine
work stays excluded.
