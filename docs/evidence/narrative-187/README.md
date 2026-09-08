# Native project and quest Flow acceptance

Recorded 8 September 2026 with the actual Tauri/WebKitGTK application, combined backend at
3db1e14 and frontend through integrated commits 666e13d / 8d6c756. The isolated Xephyr display is 1440 × 900,
100% UI scale, using Vite development assets and software rendering. The temporary harness
uses public Tauri commands for fixture setup and actual DOM focus/click/input for navigation;
Return, Escape, C and Space use trusted xdotool input. This is harness-assisted desktop proof,
not a pure Tab walkthrough, production build benchmark, or cross-platform claim. No IPC or
provider replies are mocked, and no provider is called.

## Authored structure and shared operations

The temporary three-scene fixture has two World quests with overlapping membership, three
explicit quest stages, one authored stage transition, a character participant and typed scene
exits. The shared scene appears once. Stage transitions are separate World edges; no scene
ordering is inferred from prose or quest membership.

Native Add scene created a native ID; Add first beat and keyboard C connection put an outcome
in the shared draft. Enter focused its first beat. The saved source contains that exact scene
and outcome identity. Changing an existing destination in the arc outline to Nothing yet
produced a blocker, and **Open the scene** focused the exact owning outcome's destination
select. Escape returned to the arc, retaining the draft. Undo/redo and Save retained the
result in the arc after the native projection refreshed. The adjacent target snapshot records
the scene, beat and outcome IDs and focused destination select.

[Destination blocker](arc-native-dangling.png),
[exact destination focus](arc-native-dangling-target.png),
[saved projection](arc-native-dangling-saved.png), and
[blocker retained while filtered and folded](arc-native-filtered-blocker.png).
The [short recording](arc-native-final.mp4) shows the destination/filter/return and cosmetic
collapse sequence. Fixture setup and the earlier creation sequence are not in this recording.

`arc-native-invariance.json` records identical hashes for all four narrative source files and
identical complete compiler results before/after cosmetic collapse. The compiler result contains
fixture validation findings; equality does not claim a release-ready story. Layout files are
excluded from the source hash. No temporary project is committed.

## One thousand scenes

A copy of the existing 1,000-scene / 50,000-variant fixture supplies 20 overlapping quests and
20 World stages. Its scenes have no authored cross-scene exits; native wire editing is therefore
proved with the separate small fixture above. The original scale fixture remains untouched.

- All 1,020 scenes/stages are represented. Initially 40 collapsed groups reach the React Flow
  store; only 22 were in the DOM at the captured viewport.
- Opening a 50-scene membership group yields 90 actual store nodes, including the group frame;
  a settled capture also had 90 DOM nodes. Both are below the 300-node ceiling. After a full
  reload the open group remained open: 90 store nodes / 25 DOM nodes at the restored overview.
- Opening every group invokes the announced selected-neighborhood fallback: the disconnected
  selected scene and its frame give two store/DOM nodes. This is not a claim that all 1,000
  readable cards fit on screen.
- The paged outline and all-node chooser reach scene 0999, with 25 rows per page. Full documents
  are loaded for the selected scene only; the projection contains metadata and typed exits.
- Initial Every scene action to the canvas toolbar's rendered count took 422 ms in this run.
  This is one mount observation, not a percentile or an interaction-frame measurement.

[Expanded light canvas](arc-native-1000-final-light.png),
[light outline at the final scene](arc-native-1000-outline-final-light.png).
[Verified dark readable canvas](arc-native-readable-canvas-dark.png) and
[verified light readable canvas](arc-native-readable-canvas-light.png) show the small authored arc
at a readable zoom. Adjacent `*-theme.json` files record the computed body colors: dark
`rgb(13, 14, 18)` and light `rgb(238, 235, 230)`. Earlier session theme switches had stale HMR
styling; the older functional and 1,000-scene captures are light. The verified pair was recaptured
after a full frontend reload. The recording also belongs to the earlier light session.

Raw snapshots retain an empty browser error list. The canvas overview is zoomed out; the outline
is the readable alternative when the project exceeds the node budget.

A final quiet-window sample captured 22 Space key events from 30 attempted OS inputs while
alternating focused group nodes. Event to the second animation frame was 42–48 ms, inclusive
p95 47 ms. Only observed events are counted; missing observations are not assigned zero latency.
Harness focus setup is excluded. Root Cargo/npm checks were paused; unrelated desktop processes
remained running (earlier load average 1.67). This measures keyboard dispatch to paint opportunity,
not compositor presentation or full pointer-drag performance. The raw `keyFrames` also contain
xdotool NumLock events, which are excluded from the Space sample.

## Regressions found during native proof

Native testing found four defects that mounted fixtures had missed: whole-scene Enter lacked a
focused target, saved source invalidation omitted the arc projection, scene title content could
shrink inside the node, and Rust's omitted `collapsed: false` was mistaken for an absent group.
The fixes preserve implicit-entry focus, invalidate the projection, allocate readable node height,
and default collapse only for groups missing from the sidecar. A 301-scene regression now models
the real Rust serialization and verifies an opened group stays open after acknowledgement.

Focused verification: 260 Flow/query tests passed before the final collapse regression; that new
regression and the complete 57-test mounted project Flow file pass with TypeScript, ESLint and
format checks. These tests use mocked IPC and are separate from the native evidence above.
The broader Script/Review latency work in #194/#182 remains open; this evidence does not claim
that the previously measured whole-project Review reconciliation freeze has been resolved.
