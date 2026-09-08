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

The project arrangement reads complete scene source only for its current page of 50 scenes, after
applying the selected quest scope. Previous/next controls show the range and total; connections to
scenes outside the current page are not drawn. Use the Library to find a scene directly. Opening an
individual scene does not load the arc's scene files, and hidden arc panels defer their source reads.

## Verification

Mounted-component tests use mocked IPC and exercise exact full-document parity between Flow and
Script, explicit saves, guarded writes, locked deletion, route creation and draft history. Reducer tests
check stable identities, tombstones, reconvergence, opaque field preservation and unchanged prose,
provenance and revisions. Reveal tests cover hidden tabs, repeated requests, project boundaries,
deleted targets and the empty scene. These tests do not establish native geometry or pointer behavior;
native verification of this authoring update remains pending the integrated desktop walkthrough.
