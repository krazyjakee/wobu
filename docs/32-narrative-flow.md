# Flow and outline authoring

Flow, its outline list, and Script edit one complete scene draft, keyed by project and stable scene
ID. A connection never saves unfinished dialogue. Use **Save scene** to commit the combined draft,
**Discard changes** to adopt the current saved source, and **Undo draft / Redo draft** while editing.
The successful save creates one canonical undo entry. Incoming file changes retain the original save
guard and your draft so a conflict cannot silently overwrite either version.

Select a beat, choice, outcome, ending, or scene link to inspect its typed source fields beside the
canvas or outline. Beat controls rename, reorder, duplicate and delete. Choice and outcome controls
edit destinations, conditions and ordered effects. Explicit endings have editable labels. State
comparisons and effects use the declared bool, integer and enum variables; host commands remain
structured authored effects, executed only by the deterministic runtime's host contract.

The **Add** controls create beats, choices, conditional outcomes, ordinary outcomes, and explicit
endings. **Add beat** inserts after the selected beat in both Script and Flow, or appends when no beat
is selected. Choices and unfinished outcomes start with an unresolved destination. A condition is the
guard on a route; it does not create a second story model. Several routes can target the same beat
without copying that beat or its dialogue.

**Disconnect destination** preserves the route ID, label, condition and effects, and writes an explicit
unresolved destination. Both compiler profiles reject it until connected. Deleting a destination beat
retains incoming references to its retired ID and records a tombstone. Deleting an ending or scene-link
box disconnects its owning route; deleting a route retains its beat and dialogue. Locked wording blocks
protected deletion, with Review remaining the authority for policy changes. Duplicating a beat mints
new source IDs, retains prose and provenance, and starts the copied text in Draft with Edited policy
unless it is Locked.

The outline exposes the same source operations with ordinary buttons and selects, including branch
creation, disconnect, typed fields and deletion. Announcements report refusals and successful changes.
Latched reveals wait for a visible target, expand a containing collapsed group when needed, and retain
stable route IDs across tabs. Deletion returns keyboard focus to a surviving beat or the empty scene's
creation control. Typed diagnostic requests focus the responsible field instead of competing with node
focus.

Arrangements remain cosmetic: positions, groups, collapse, notes, automatic/manual mode and quest
sidecars use the independent persistence described in [Narrative layout](27-narrative-layout.md).
Automatic layout never rewrites source, accepted text or compiler output. The React Flow store still
contains at most 300 nodes **including group frames**, with at most 40 visible annotation overlays;
paging and focus neighborhoods expose the rest. Group and note controls remain paginated. The
outline shares arrangement mode, retry, group membership/rename/collapse/delete, participant, text
status and badge controls with the canvas. Pinned notes appear as a paged list with the same text and
delete controls, plus numeric placement fields. Reveals expand folded groups before focusing their
members. Presentation and filtering never create a scene draft or a source undo entry.

**Groups & notes → Selected node position** offers numeric X/Y placement in both canvas and outline.
Entering a coordinate switches the arrangement to Manual and retains the companion coordinate from
the displayed canvas or the outline's seed arrangement. Read-only scenes permit temporary automatic
layout without attempting to write a sidecar.
Deleting a focused destination edge returns focus to the destination field in a saved scene, or to
the surviving route in the standalone demonstration.

At narrower workspace widths, including UI scaling, **Scene outline** and **Context** open auxiliary
panes above the editor. Escape closes a pane and returns focus to its button. Selecting an outline
beat closes the pane and reveals that beat. The validation explanation is available under
**About validation and affected text**, leaving room for the canvas and authoring controls.

The arc reads one content-checked, prose-free project projection. Every authored cross-scene
choice/outcome retains its route and owning beat ID, including unresolved destinations. World
quest stages appear as separate nodes with exactly their authored transitions. Stage membership
never invents a scene-to-stage edge, an entry scene, or a runtime scene order. Broken transitions
link to their quest in World; scene destination diagnostics open the owning route field.

**By quest** is the initial grouping. A scene listed by several quests appears once in a combined
membership group. **By quest state** groups by authored initial stage, since runtime quest state
belongs to a player's session. Each grouping has a separate cosmetic arrangement, including collapse;
**By arrangement group** restores explicitly drawn groups. Layout format 3 adds quest-stage keys;
versions 1 and 2 remain readable without rewriting them, and older peers refuse version 3 safely.
Stage rename changes its presentation key because the authored World model identifies stages by name.

Canvas and outline create scenes through the same native command as the Library. Select a scene to
read its source and edit its existing exits or add an outcome on an explicitly selected beat. Empty
scenes offer **Add first beat for scene exits**. These edits use the shared Script draft, unsafe-integer
and locked-text guards, explicit Save, draft undo/redo and guarded persisted undo. Quest stages are
edited in World. Source-read failures and destination blockers remain named even when nodes are
filtered, folded or outside the viewport.

The canvas hands React Flow at most 300 nodes, using collapsed groups and a selection neighborhood
when necessary. The outline offers 25 rows per page, an all-node chooser, and editable destinations
for the selected scene. Participant filters use entity IDs with names as labels; work counts read real
text lifecycle state and include filtered or collapsed scenes. Enter/double-click drills into a scene;
Escape and the breadcrumb return to the saved arc scope, grouping, cursor, viewport and outline page.
Opening one scene does not fetch every scene document; hidden arcs defer the compact projection.

## Verification

Mounted-component tests use mocked IPC and exercise exact full-document parity between Flow and
Script, explicit saves, guarded writes, locked deletion, route creation and draft history. Reducer tests
check stable identities, tombstones, reconvergence, opaque field preservation and unchanged prose,
provenance and revisions. Reveal tests cover hidden tabs, repeated requests, project boundaries,
deleted targets and the empty scene. The mounted canvas regression creates three routes, connects
each to one verdict with C, disconnects a focused edge, checks focus on the destination field, and
undoes the disconnect in Script while retaining all dialogue. Presentation tests also assert numeric
outline placement and read-only layout cannot write scene source. These tests do not establish
native geometry or pointer behavior. [Native Linux evidence](evidence/narrative-186/README.md)
records both themes, visible minimap nodes, and keyboard creation/deletion with retained focus at
150% scaling. Reveals preserve the current zoom and wait for automatic layout to finish before
focusing a node. The footer checks the same unsaved scene shown in Flow, rather than reporting
the saved source's problem count beside a changed draft.

Additional mounted parity cases compare Add beat, Duplicate beat and Delete beat through both views,
normalizing only freshly minted identities and deletion timestamps. They check selected-anchor order,
locked text and provenance, preserved incoming references, tombstones and shared undo/redo. A
three-choice reconvergence case compares the complete resulting source and restores all routes with
Script undo while retaining locked dialogue.
