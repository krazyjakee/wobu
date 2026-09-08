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
endings. Choices and unfinished outcomes start with an unresolved destination. A condition is the
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
Entering a coordinate switches the arrangement to Manual; an unset companion coordinate starts at
zero. Read-only scenes permit temporary automatic layout without attempting to write a sidecar.
Deleting a focused destination edge returns focus to the destination field in a saved scene, or to
the surviving route in the standalone demonstration.

At narrower workspace widths, including UI scaling, **Scene outline** and **Context** open auxiliary
panes above the editor. Escape closes a pane and returns focus to its button. Selecting an outline
beat closes the pane and reveals that beat. The validation explanation is available under
**About validation and affected text**, leaving room for the canvas and authoring controls.

The arc's **Group** control carves the same scenes up without editing them. *By arrangement group*
shows the groups drawn in the sidecar and is what a project arc opens on. *By quest* and *by quest
state* read `narrative/world.yaml`: a scene sits in the quest whose `scene_ids` name it, and a
quest's state is the stage it starts in, because a project records no running quest state. A scene
listed by several quests appears in the first and the pane says how many are affected. Quest groups
collapse and expand on the canvas and in the outline, but they last for the session only: the
arrangement file keeps the collapse of the groups a writer drew, not of a grouping derived from
World. Switching the control writes nothing to source, to World or to the arrangement. When World
cannot be read the quest groupings are refused with that reason instead of grouping by something
else.

The project arrangement reads complete scene source only for its current page of 50 scenes, after
applying the selected quest scope. Previous/next controls show the range and total; connections to
scenes outside the current page are not drawn. Use the Library to find a scene directly. Opening an
individual scene does not load the arc's scene files, and hidden arc panels defer their source reads.

## Verification

Mounted-component tests use mocked IPC and exercise exact full-document parity between Flow and
Script, explicit saves, guarded writes, locked deletion, route creation and draft history. Reducer tests
check stable identities, tombstones, reconvergence, opaque field preservation and unchanged prose,
provenance and revisions. Reveal tests cover hidden tabs, repeated requests, project boundaries,
deleted targets and the empty scene. The mounted canvas regression creates three routes, connects
each to one verdict with C, disconnects a focused edge, checks focus on the destination field, and
undoes the disconnect in Script while retaining all dialogue. Presentation tests also assert numeric
outline placement and read-only layout cannot write scene source. These tests do not establish native geometry or pointer behavior;
native verification of this authoring update remains pending the integrated desktop walkthrough.
