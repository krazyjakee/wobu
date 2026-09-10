# Bounded native verification of the read-performance slice

Measured 8 September 2026 through the actual Linux Tauri/WebKitGTK bridge, with the synthetic
1,000-scene / 50,000-slot scale project. This verifies the current implemented slice; **#194 and
#182 remain open**. The [remaining-criteria audit](../current-acceptance.md) distinguishes existing
focused evidence from the complete workflow still outstanding. N5 is excluded.

Backend production files match `cbeb9bf` (built from `d3c403b` plus the subsequently committed
`media-src blob:` CSP change; intervening `9685375` changed tests). Frontend product files match
`f331534`. Native PID 3750426, isolated DISPLAY `:97`, reported window `0x200003`, 1440 × 900,
scale 1. The build log is `/tmp/wobu-current-batch3-native-blob-probe.log`; process log is
`/tmp/wobu-acceptance-20260908/native-current-batch3-blob.log`. No IPC replies, provider outputs or
canonical source reads were mocked. No provider calls were made.

## Method and limits

A temporary Vite harness invoked public commands and observed `requestAnimationFrame` intervals.
The [driver](measurement.py), [harness operations](harness-operations.txt) and
[summary calculation](summarize.py) retain the measurement method. They are evidence tools, not
production code or a standalone automated desktop setup. Native read measurements ran while the
frontend showed its Art workspace, without an active Script Review request. This isolates IPC
scheduling from mounting the narrative editor; it does not establish full interactive UI latency.

Root Cargo/npm checks were stopped and other agents did no heavy checks. Unrelated desktop/user
processes remained active; [read environment](reads-environment.json) and
[action environment](actions-environment.json) record load/process snapshots. Process CPU percentages
are lifetime averages, not instantaneous attribution. Rust uses the debug development build.
Project open used an existing disposable project-ID-local index; disk/index cache temperature was
not controlled, so its single 7,982 ms result is not labelled cold-index or cold-disk performance.

All 30 samples per read group and all outliers remain in [raw samples](read-samples.json).
The [summary](read-summary.json) uses inclusive interpolated p95. Each search requested a distinct
exact phrase and verified its stable scene ID, 50 matches and at most five snippets. Combined
wording + quest + edited-policy queries verified unique IDs, exact quest membership and at most
25 rows. Scene reads verified the selected ID; Review returned all 50 lines. Exact canonical
hashes [before](read-source-before.json) and [after](read-source-after.json) match across the read
phase. The full temporary synthetic project was not added to the repository.

Frame intervals include two callbacks after each IPC result; the first begins at timer installation
rather than the prior display frame. Summed process-tree RSS includes native/WebKit descendants and
duplicates shared pages; it is not PSS or a continuous sampled peak. The retained-growth-after-50-cycles
budget was not tested. [Predefined budgets](../profile/budgets.json) were not changed after observing
results.

## Results and missed budgets

| Current public IPC group | Samples | Median ms | p95 ms | Maximum ms | Assessment |
| --- | ---: | ---: | ---: | ---: | --- |
| Exact wording search | 30 | 205.0 | 288.7 | 405 | Misses 250 ms target |
| Combined wording + quest + policy | 30 | 233.5 | 286.0 | 341 | Misses 250 ms target |
| Selected scene get | 30 | 70.0 | 110.7 | 121 | IPC observation; does not prove UI open budget |
| Review, 50 slots | 30 | 437.5 | 523.0 | 734 | Misses 500 ms target |

All read groups had frame-interval p95 of 17 ms; the maximum across read calls was 21 ms.
Read-phase sampled native/WebKit RSS peaked at **791,076,864 bytes** (about 754 MiB), below 1 GiB.
This substantial improvement over the earlier native Review observations near 20–21 seconds does
not make the remaining latency targets pass. The separate store profile isolates the repeated
parsing improvement; these integrated observations are not a claim that every difference is caused
by one code change.

The existing guarded write sequence succeeded through actual public commands. The
[action observations](action-samples.json) retain target IDs, guards, approval/policy status,
text revision, history counts, timings and frames; full source replies remain only in temporary raw
artifacts. Approve returned valid approval; lock/unlock retained that approval and text revision.
A guarded summary-only save then succeeded. No before-change write baseline was measured, so these
waits are **not classified as new regressions**:

| Action | One observation, ms | Maximum observed frame interval, ms |
| --- | ---: | ---: |
| Approve | 31,426 | 22 |
| Lock variant | 24,134 | 18 |
| Unlock variant | 23,034 | 21 |
| Guarded scene save | 27,143 | 18 |

[RSS after those actions](action-memory.json) was **1,088,008,192 bytes**, slightly above the
1 GiB budget. A passing read-only memory sample does not establish overall memory acceptance.
The long successful action waits and post-action memory miss remain explicit follow-up criteria.

## Short DOM integration observation, not visual acceptance

After the actions, a frontend reload selected the actual scale project. One Library search found
the exact phrase in 269 ms. A Script textarea containing that phrase appeared in 711 ms, including
a **525 ms frame interval**; return detected the retained matching Library table in 64 ms.
These are single DOM-availability observations, not 30-sample UI p95 measurements. The helper did
not require the matching element to be painted/visible; return finished before a measured frame
callback and therefore has an empty frame array, not proof of a zero-duration stall.

The [settled DOM record](ui-final-settled.json) contains the restored matching row, selected stable
IDs and no JavaScript errors. Its active element was a retained hidden textarea. Activation used
harness `.click()`/input events, so this does not claim keyboard focus restoration. Two temporary
native PNGs captured an identical stale “Reading the script…” frame despite populated DOM snapshots;
they are intentionally excluded from visual acceptance evidence. [Computed theme state](theme.json)
was genuinely dark (`rgb(13, 14, 18)`), but a theme value cannot establish successful painting.

The visible-open/return, keyboard focus, rendering stall and full memory/cancellation/workflow budgets
remain open. No additional product implementation was started after these bounded observations.
