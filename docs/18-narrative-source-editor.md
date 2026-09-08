# Narrative Source editor

The Source tab reads the scene's exact YAML bytes and the file stamp from the same read. Validate
parses the draft into the shared typed scene model and shows syntax/type errors with line and column
when the parser provides them. Scene diagnostics link to the corresponding beat in Flow or Script.
These links use stable IDs; semantic diagnostics do not yet carry exact YAML source ranges.

Save is explicit. It uses the same guarded scene write and undo history as Script and Flow. A scene
ID cannot be changed through this editor. Unknown fields and unsupported schema versions are
rejected before a draft can reach the save command. Parsing and validation do not write files or
change compiled assets. Structural diagnostics can remain in an unfinished scene, as in the other
editors; they still need to be addressed before production compilation.

The canonical stored representation is the typed scene's YAML serialization. Reading preserves
comments and formatting in the editor. **Format and Save normalize formatting and remove comments.**
The pane states this policy beside the controls. IDs, Unicode, multiline text, and supported model
fields survive a round trip. Layout stays in its separate presentation sidecar and is never embedded
in scene YAML. Form edits serialize that same model; there is no parallel document authority.

Unsaved drafts survive switching tabs, scenes, and workspaces in the running session. Drafts are
scoped by project path and scene ID and retain the original file stamp. A later form or external edit
therefore causes a conflict instead of silently replacing the latest source. A failed save keeps the
draft. Reload asks the writer to discard a dirty draft explicitly before loading the latest saved
version. Closing the project or quitting is blocked until unsaved narrative drafts are saved or explicitly discarded, even when the Source tab is no longer mounted. Drafts are not persisted across application restarts.

Changing dialogue directly requires a matching content revision before it can retain approval; the
shared write boundary enforces the review contract. Script is the convenient way to author prose
with a computed revision. Source validation also reports mismatched revisions in unfinished drafts.

The initial Source load currently requires a parseable on-disk scene. Repairing an externally broken
file that cannot be identified or parsed remains a file-editor task; this UI supports malformed
*unsaved drafts* and will not write them over valid project source. A syntax-coloured editor and
precise semantic source ranges remain follow-up work for #157.

Validation covers exact-byte reads, comment normalization, stable identities, Unicode and multiline
text, unsupported fields/versions, malformed drafts, cursor navigation to syntax errors, retained
preconditions across tab changes, conflict retention, project-scoped drafts, and read-only controls.
Frontend checks use jsdom with mocked commands; Rust command tests use real temporary project files.
