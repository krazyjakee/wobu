# Native World, conflict and repair checks

Captured 2026-09-08 in the actual Tauri application with WebKitGTK, inside an isolated
1440 × 900 Xephyr display on Linux. These checks use the Light theme at 100% scale.
The temporary acceptance projects contain only deliberately created test content.
No IPC responses or provider results were mocked, and no provider was called.

The temporary loopback harness called public Tauri commands for fixture creation, reads,
the concurrent writer and opening projects. It also focused named controls and selected
tabs. `xdotool` sent the actual typing, Return and Ctrl+Z events. This is harness-assisted
native interaction, not a claim that every control was reached by Tab. JSON files record
the actual DOM, focused element, selection and browser errors beside each screenshot.

## World and entity backlinks: passed

[The 86.5-second recording](world-proof-keyboard.mp4) covers both entity types and the
World record:

- A fact refers to Captain Irena, a Character, and Council Chamber, a Setting/place, by ID.
  Opening either related entity focuses its **Narrative records (1)** disclosure.
- Native typing renames the character to Captain Irena Vale and the place to Old Council
  Chamber. Entity autosave completes. The backlink opens the same World fact, focuses its
  Record name input, and displays the renamed entity.
- Ctrl+Z from a non-input control restores each original entity name. Both IDs and the
  fact's related-entity references remain unchanged.
- Renaming the fact to Hearing testimony and pressing **Save world** updates the label
  shown in the entity's backlink. Returning through it and pressing Ctrl+Z restores the
  exact original World document through guarded undo.

Representative captures: [character backlink return](world-proof-character-return.png),
[place backlink return](world-proof-place-return.png),
[renamed World record backlink](world-proof-record-backlink.png), and
[World undo](world-proof-record-undone.png). The recording includes one Ctrl+Z while an
input has focus; the successful application undo follows after focusing **Facts**.
[Public-command checks](public-command-checks.json) record stable IDs and source equality.

## Stale scene save and retained draft: passed

[The 29.4-second recording](conflict-proof-keyboard.mp4) creates a local Script name draft.
A separate public `narrative_scene_save` with the current stamp changes the saved summary.
The Script Save then refuses its stale stamp, identifies the retained conflict sibling,
and keeps the local draft. A second save with the short name **Local draft** confirms it
again. The concurrent writer's canonical name and summary remain intact.

The draft and refusal survive switching to [Flow](conflict-proof-flow-retained.png) and
[back to Script](conflict-proof-script-return.png). [The refusal](conflict-proof-retained.png)
shows the retained sibling path. After capture, the draft was explicitly discarded and
the original summary restored through a fresh guarded public save; its editorial head
advances normally. The existing Ashfall route recordings were not modified.

## Recorded-source repair: baseline failure found

[The 19.25-second recording](repair-proof-keyboard.mp4) opens the actual malformed-file
repair UI, removes an appended unterminated YAML line using native keyboard editing,
and runs Validate. [Validation accepts the draft](repair-proof-valid-draft.png), but the
baseline binary's [Save source refuses its existing editorial head](repair-proof-refused-baseline.png).
The malformed canonical file and repair draft remain intact; this recording does not
show successful repair.

Commit `41fe6f0` fixes this narrowly using the original intact canonical identity/head
header and the exact verified immutable receipt snapshot. Real-file command tests cover
repair of approved, locked wording, unchanged receipts, exact recovery bytes, reopening
and valid approval, plus refusal of changed text/policy/context, substituted history,
tampered receipts and missing identity bindings. Native success must be recaptured with
a rebuilt binary before being claimed.

These captures supplement the [both-theme and 150% Flow evidence](../narrative-186/README.md).
They do not establish native read-only coverage, all #154 states, #194 performance budgets,
or completion of the full Ashfall acceptance walkthrough.
