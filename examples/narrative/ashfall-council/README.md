# Ashfall council hearing

This original handwritten example extends the council/logbook scene used by the
native Preview walkthrough. It contains three choices that reconverge on one
response beat, an authored trust effect, knowledge-sensitive choices/wording,
and an explicit host command. No provider or game engine is used.

`source.json` is the shared authoring template consumed by the frontend and
Rust command regression. Its placeholder text revisions must be sealed by
`narrative_text_written` before saving. It is not a packaged runtime asset.
The frontend mints fresh stable element IDs for each creation and preserves
those IDs during later saves and reopen.

In Narrative, select **Open Ashfall example**, then **Create example draft**.
This saves three prefixed declarations and creates an empty scene using the
ordinary public commands, then opens a shared Script/Flow draft. Choose
**Save scene** to seal and save its prose. These are separate, ordinary undo
steps. Existing compatible variable declarations retain their defaults and
descriptions; incompatible types or owners are refused before writes. An
unfinished variable draft blocks creation. If a later command fails, earlier
writes remain visible and the dialog explains how to inspect them.

In Preview, register `ashfall_file_record` with no arguments. Use these states:

| Trust | Knows logbook | Choice | Result |
| --- | --- | --- | --- |
| 40 | true | Present the logbook | Trust 50; clerk command; return `ashfall_record_filed: true` for acceptance |
| 20 | true | Ask for a fair hearing | Trust 25; clerk asks to see the logbook |
| 20 | false | Challenge the council’s silence | Trust 10; council asks for a witness |

All paths end at **Hearing complete**. The knowledge value is an explicit host
input for this compact sample; it does not claim an engine integration or a
new epistemic storage mechanism.

Verification layers remain separate: frontend tests use mocked public IPC and
exercise creation, explicit Save, reopen, partial failures, and project-session
changes. The Rust regression uses real files and guarded command helpers,
reopens exact saved source/IDs, and drives public Preview commands through the
three states above. A native WebKit walkthrough remains required for #194;
this fixture and its tests do not establish that acceptance on their own.
