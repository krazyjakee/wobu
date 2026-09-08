# Native Flow focus and layout verification

Recorded 8 September 2026 in the actual Tauri/WebKitGTK application, using Vite development
assets and the product components in this change. The isolated Xephyr display is 1440 × 900;
this is Linux software-display validation, not a physical-display or cross-platform claim.
The temporary acceptance harness sends public commands through the real Tauri bridge and reads
the actual DOM. No IPC replies or provider results are mocked, and no provider is called.
Only the temporary `narrative-acceptance.wobu` fixture is used.

The [keyboard recording](flow-proof-final.mp4) starts in Automatic arrangement at 150% UI scale.
Each sequence starts with harness focus on the Flow tab, then uses trusted OS Tab, Return and
Delete input through xdotool. Scale/theme switches use the product settings store through the
test harness; those setup actions are not represented as keyboard interaction.

- Tab to Add beat and Return creates and focuses a new beat **after ELK finishes**, retaining
  the current graph zoom. The canvas scrolls into the outer panel's visible area. The new node
  and footer both report the unresolved structural problem in the current unsaved draft.
- Delete removes that new beat and focuses the surviving Council response node. No BODY-focus
  fallback occurs. These source edits stay in the shared draft; they are not saved by a layout.
- At 100%, Tab/Return invokes Fit in both themes and switches to Outline list. The minimap
  contains visible node rectangles and its viewport mask in both themes.

[Created beat at 150%](flow-proof-create-150.png), [surviving focus after deletion](flow-proof-delete-150.png),
[light canvas](flow-proof-light.png), [dark canvas](flow-proof-dark.png), and
[dark outline](flow-proof-outline-dark.png) retain the corresponding raw DOM/selection/error
snapshots in adjacent JSON files. All recorded snapshots contain an empty browser error list.
The rename fix was separately exercised natively: Return on the Library Rename button focuses
its input, and Escape returns focus to that row's Rename button.

The native failures that led to this change were a reveal defaulting to React Flow's maximum
zoom, an ELK result moving a node after its reveal had been consumed, and the Library rename
form replacing its focused trigger without focusing the input. Mounted tests now guard explicit
zoom preservation, deferred automatic-layout focus, diagnostic-only geometry stability, rename
entry/cancel/save focus, and current-draft footer diagnostics.

This focused recording does not establish every narrative workflow or scene-library performance
budget. Full three-route typed authoring and Script undo parity are covered separately by mounted
component tests with mocked IPC; broader native World, conflict, search and Preview acceptance
belongs to the integrated workspace evidence.
