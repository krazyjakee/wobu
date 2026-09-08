# Scene-library scale evidence

This directory records #194's real filesystem and later IPC measurements. A store measurement is
not a native UI result. The complete issue remains open until the integrated workflow and its
performance budgets have been exercised.

## Reproduce the source fixture

From `src-tauri/`, run:

```sh
cargo run -p wobu-store --example narrative_library_fixture -- /tmp/wobu-scale-run-01
```

Choose a new parent directory each time. The tool refuses to overwrite an existing project. It
creates 1,000 typed scenes with 50 dialogue slots each, stable narrative IDs, synthetic draft wording,
explicit endings and 20 overlapping quests. It writes canonical YAML directly for scale setup;
this is separate from the original Ashfall public-command authoring walkthrough. There are no
provider calls or approval receipts, and this fixture does not establish Release readiness.

The tool removes only this newly created fixture's initial empty local index, then measures opening
and rebuilding it, listing scenes and eight subsequent reads of one selected scene. Source stays
intact. The files were just written, so this is a **cold index with a likely warm filesystem cache**,
not a physical-disk cold-read experiment. Each run prints JSON with its scope and timings.

## Baseline before indexed library access

[Raw measurements and machine details](baseline-store.json) use the unoptimized development build
at merged PR #198, commit `2b31ac1`. The fixture contains 22,406,000 bytes of scene YAML and exactly
50,000 dialogue slots.

| Store operation | Baseline |
| --- | --- |
| Open with an empty index | 6,852.6 ms |
| Read the scene catalog | 2,484.5 ms |
| Read one selected scene, eight subsequent samples | 2,454.6–2,611.3 ms |

At this revision, `Project::load_scene` calls `scene_catalog` to locate each scene before reading
it. The catalog reparses the entire corpus. The frontend's existing `useSceneFiles` requests every
scene, multiplying that work rather than merely producing a large response. The full native
1,000-scene view was therefore not launched for this baseline; these numbers establish the store
problem, not an observed native timeout or frame rate.

The indexed implementation below uses bounded library discovery and scene-ID lookup while retaining
authoritative guarded reads of selected source. The complete UI verification will
measure search/filter and open/return separately, including IPC count/bytes, mounted rows, memory
and main-thread stalls. Initial targets are warm search/filter p95 ≤250 ms and cached open/return
p95 ≤200 ms on the documented reference machine. The UI budgets remain unverified.

## Indexed native IPC checkpoint

[Raw native measurements](indexed-native-ipc.json) use commit `c536e8b`, Rust development profile
and WebKitGTK 2.52.6 on the same machine. The fixture is the exact original version-1 YAML dataset,
not a smaller in-memory substitute. All 1,001 canonical YAML hashes matched before and after.

The temporary harness called the registered public commands through the real Tauri bridge in
WebKit. It opened the project with fresh private XDG directories, requested a 25-row library page,
and then ran 30 samples each of browsing, remembered-line search, combined filters and selected-scene
read. Search verified the exact stable scene ID and 50 matches with only five snippets returned.
The native window stayed outside the unfinished large-project Narrative UI. The harness and dev-server
reporting proxy were removed afterward; there were no provider calls or source writes.

| Real IPC operation | Samples | p50 | p95 | JSON reply bytes |
| --- | --- | --- | --- | --- |
| Browse 25 scene summaries | 30 | 158 ms | 177 ms | 13,999 |
| Remembered dialogue phrase | 30 | 186 ms | 209 ms | 3,677 |
| Text search + quest + Edited | 30 | 211 ms | 234 ms | 16,948 |
| Read one selected scene | 30 | 65 ms | 84 ms | 18,693 |

The first project-open command took 19,289 ms, including creation of the larger derived text index.
This is a single cold-index observation with warm source filesystem cache; process/portal startup
is excluded. The first library page took 160 ms. Reply sizes are serialized JSON bytes, excluding
transport overhead. Percentiles use nearest rank and retain all samples.

These results meet the initial **backend** search/filter budget. Selected-scene IPC is also below
200 ms, but that does not establish cached UI open/return performance. Native rendering, mounted
rows, memory, frame stalls, retained scroll and the complete keyboard workflow still require the
integrated Library/Flow/Script walkthrough.

For a reproducible store-only comparison on this fixture, run:

```sh
cargo run -p wobu-store --example narrative_library_query -- /path/to/narrative-scale.wobu
```

That example reports filesystem/store timings, independently of the native measurements above.
