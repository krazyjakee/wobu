# Narrative authoring: current implementation

Merged PRs [#190](https://github.com/krazyjakee/wobu/pull/190) and
[#192](https://github.com/krazyjakee/wobu/pull/192) established Flow, scene discovery, handwritten
Script and YAML Source. The current implementation adds attributed world records, typed scene
forms, a deterministic Rust compiler/runtime and playable Preview. These are working foundations;
the [delivery plan](17-narrative-system.md) records the acceptance still outstanding. N5 engine
integrations (#173–#177) are excluded from this work at the user's request.

## Find a scene

Narrative opens a paged scene table. Search matches scene names, summaries, beat titles, authored
intent, required content and dialogue. Each matching passage carries scene, beat, slot and variant
identity into Flow or Script. Generated draft text is an explicit search option. Quest, participant,
policy, approval, freshness and missing-text filters combine; policy/approval/freshness must match
the same variant. Values are the recorded source values, not results of a dependency build.

A quest owns an explicit list of scene IDs. A scene can belong to several quests and appears once
in the table, with every membership in its Quests column; either quest filter finds it. Set those
memberships in World state → Quests. Renaming a quest preserves saved filters because they use its
ID. A saved filter for a deleted quest stays selected with an explanation until changed or cleared.

Saved views, pinned/recent scenes, query, sort, page, selected row and scroll position are local to
the project path on this machine. Older saved views acquire an empty quest filter. Back to scenes
retains the view. Only 25 result rows are mounted; scene documents still load through existing
queries, so this is not an indexed search backend. Act and tag metadata remain unimplemented.

The earlier library screenshots show the base discovery layout before the quest column was added:

![Scene library, dark](screenshots/narrative-library-dark.png)

[Light-theme library](screenshots/narrative-library-light.png).

## Establish world knowledge

Open **World state** to edit facts, knowledge, relationships, events, quests, future knowledge and
variables. A fact records its canonical assertion and sources. Related world entities link facts
and events to existing characters or places. A knowledge record separately identifies its character,
fact, belief, provenance and acquisition condition. For the same attack fact, Kael can have witnessed
it, Mira can have been told by Kael, and Orren can believe a contradictory rumour. Recording false
or unknown belief never rewrites the canonical assertion.

Relationships are directed: Kael trusting Mira does not manufacture a reciprocal relationship.
Events reference established facts and an applicable condition. Quests declare their stages,
initial stage, conditional transitions and scene memberships. Future knowledge restrictions name
a fact and the characters who must not reveal it until a condition holds. An empty character list
means everyone; **Never** keeps the restriction in force indefinitely. These records author intent
and constraints; automatic attribution into generation requests remains #163.

**Variables** declares the finite domains used by conditions: booleans, bounded integers and named
values. Names use lowercase letters, digits and underscores, starting with a letter; reserved YAML
words such as `true`, `false` and `null` are not names. Narrative-owned variables may be assigned by
story effects. Host-owned variables are read by the story and supplied by its host. Changing or
removing declarations can invalidate existing conditions; renaming does not rewrite references.

**Save world** writes `narrative/world.yaml`; variables stay in `narrative/state.yaml`. World records
retain stable IDs through renaming and undo. Semantically incomplete records, such as references to
removed facts, can be saved and return record/field diagnostics. Malformed identities, invalid name
syntax, unknown fields and unsupported versions are rejected. Local unsaved drafts are retained
when a save fails. The detail pane lists reference users before deleting a referenced world record;
removing it retains those users and exposes their dangling references.

![Narrative world editor](screenshots/narrative-world.png)

## Write a scene

Script edits the same scene as Flow: scene metadata and participants; beat add, reorder, duplicate
and delete; intents and required/forbidden content; dialogue slots and variants; choices, automatic
outcomes and explicit destinations. Typed controls now edit scene entry, variant, choice and outcome
conditions. Variable/operator/value pickers use the declared schema; effect rows edit assignments,
increments and host commands. Outcomes and variants retain authored priority order. The first
matching variant is selected; available choices take precedence over automatic outcomes, whose
first matching entry is used. See the [runtime contract](17-narrative-runtime-contract.md) for the
precise evaluation and effect rules.

Save script computes wording revisions in Rust, records human provenance, resets changed wording
to draft and preserves recorded freshness. Manual slots start Edited or Locked. Locked text must
be unlocked before editing. Duplication allocates new identities while preserving wording
provenance/revisions. Guarded saves and structural changes enter the shared undo history. The
inspector displays saved participants, intent, restrictions and selected dialogue. The World editor
is the place to author attributed facts; the inspector does not yet resolve epistemic context.

![Typed Script editor](screenshots/narrative-typed-script.png)

YAML edits the same model through [Source](18-narrative-source-editor.md). Explicit formatting/save
normalises YAML, including comments; semantic diagnostic links use stable IDs rather than exact
source ranges.

![Source editor](screenshots/narrative-source-dark.png)

## Play and inspect

1. Save the scenes and variable declarations used by the scenario, then open a scene's **Preview**.
2. Set its **Starting state** using the declared variable domains. Preview can override narrative
   defaults and host inputs for this isolated run without editing project canon.
3. If authored effects use host commands, declare their argument signatures under **Host command
   signatures**. For example, `{"camera_closeup": ["bool"]}` registers one boolean argument.
4. Select **Start preview**. Saved project scenes compile into an internal deterministic graph.
   Compilation diagnostics link back to Script; structural/type errors prevent starting. Development
   warnings permit incomplete text to compile, but reaching a slot without a matching variant stops
   playback with a no-match error.
5. Read lines with **Continue**, choose available responses, and inspect the current state and
   playback history. **Open this line in Script** follows the yielded scene/beat/slot IDs.
6. Use **Save snapshot** and **Restore snapshot** to revisit a checkpoint, or **Restart preview** to
   compile the latest saved source and use the starting inputs again.

A command pauses Preview and displays its name and arguments. Return success with typed host
values, failure, or cancellation without performing a game action. Failed and cancelled commands
remain pending for an explicit retry; restoring a pending checkpoint preserves command identity.

The running session retains its compiled graph. Source edits do not change it mid-play; restarting
compiles again. Checkpoints and playback history survive view changes within the app session and
are scoped to the project/scene. They are not persisted scenario assets (#162). Playback history
shows evaluated conditions, transitions, effects before and after, and command results with source
links. Each action retains at most 2,048 trace records and reports omissions explicitly; the UI keeps
100 actions. Flow route overlays remain #188.

![Playable Preview](screenshots/narrative-preview.png)

The compiler and runtime are pure Rust crates independent of Tauri, providers and the authoring
store. Preview commands adapt them to the app without writing world/project state. This internal
graph can be exported as a [validated native package](20-native-narrative-packages.md). Engine
adapters remain excluded N5 work. There is no generation call in compilation or playback. Release validation rejects missing required text
and wording whose recorded approval/freshness is not ready; dependency freshness recomputation is
still #168. First-match selection is deterministic; a seed is retained for a future selection policy. The pure
Rust runtime supports signed 64-bit integers; the JavaScript Preview bridge rejects values or
declarations outside JavaScript's safe-integer range so IPC cannot silently round them.

## Protect drafts and saves

Script, Source and World/Variables drafts survive view changes within the session and retain their
original file stamps. A later save from another writer produces a conflict rather than silently
replacing newer work. Late save completion cannot clear a newer reopened draft. Closing a project
or quitting requires retained drafts to be saved or discarded, including drafts in hidden views.
Drafts are not crash-persistent. Source can open malformed scene files through the Library repair
action. Repair preserves the exact original in an immutable recovery file before guarded
publication; future source versions remain read-only. Canonical YAML uses mappings for nested
conditions and effects, while existing tagged source remains readable.

World undo/redo compares the expected whole document under the project lock and then performs a
guarded write using its current stamp. It refuses a changed document from another writer. Ordinary
World saves cannot request an unguarded `Current` precondition. Existing scene undo retains the older `Current` limitation documented by PR #190. Variable edits
use guarded stamp-based saves but do not yet enter undo history; shared recovery hardening remains
#153/#181.

The scene save command rejects approved wording with a mismatched revision and changed wording
that carries approval forward during an ordinary edit. Correctly sealed undo snapshots can restore
previous approval. This is an authoring safeguard, not the full revision-aware review/receipt system
in #165. World source participates in the narrative fingerprint; moving Flow boxes does not.

## Evidence and remaining acceptance

Screenshots use the actual React components with explicit in-memory IPC fixtures in Chromium.
They establish browser rendering. The separate [native Preview walkthrough](21-native-narrative-preview.md)
records real Tauri/WebKit and Rust IPC, including command results, restore, bounds and loop failures;
its canonical project-file audit verifies isolation. Rust command tests use temporary project files. Current regression coverage includes attributed
three-character knowledge, contradictory/unknown belief, entity backlinks, strict source/version
checks, guarded world saves and undo, typed forms, multiple quest membership and saved views,
compiler release gates, runtime yield/snapshot boundaries and Preview's isolated state.

The earlier #192 benchmark uses 1,000 scenes and 50,000 slots already loaded in memory. Its initial
run found a remembered line in 14–32 ms; it does not establish SQLite, filesystem, IPC or native
frame-time performance. Full current check totals belong in the implementation PR's verification
record, rather than reusing #192's counts for the changed implementation.

| Issue | Implemented in this increment | Still outstanding |
| --- | --- | --- |
| #155 | World model/forms, provenance, restrictions, quest stages/membership, related entities, diagnostics and guarded world undo. | Complete character/place backlink navigation and full native workflow acceptance. |
| #156 | Typed conditions/effects, automatic outcomes and variant controls alongside handwritten authoring. | Full public-command fixture/walkthrough and all structural undo/Flow equivalence acceptance. |
| #158 | Deterministic validated graph, source maps, typed effects/command signatures and development/release text gates. | Full acceptance review; wider analysis belongs to #170/#171. |
| #159 / #195 | Pure runner, bounded typed execution, command protocol, version/hash-checked snapshots and explicit validated migration callback. | No random variant-selection policy is authored yet. |
| #161 / #195 | Isolated Preview, starting state, checkpoints, evaluated traces, configurable command results and recorded native walkthrough. | Saved scenarios (#162) and Flow route overlays (#188). |
| #191 | Real multiquest filtering/column and migration of saved views. | Act/tag metadata, rebuildable index and real-load/native acceptance. |

Source repair and semantic ranges (#157), native packages (#160), and migration/Preview traces
(#195) are implemented. Indexing/sync/recovery (#153), persistent scenarios (#162), generation/review (#163–#167), incremental analysis
(#168–#172), Flow overlays (#188/#189) and production work (#178–#183) remain tracked. N5
(#173–#177) is deliberately excluded; no Unity, Godot, Unreal or Yarn integration was added.
