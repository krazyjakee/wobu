# Acceptance evidence

This page records what the repository can prove about the M1 and M6 acceptance
passes. It deliberately separates repeatable contract evidence from checks that
need a running desktop app, a configured external application, or live provider
credentials. Passing a unit test is not described as having clicked through a
real provider.

## M1 — structured world notes

| Acceptance claim | Repeatable evidence |
| --- | --- |
| A project is a normal local folder with the documented layout. | `wobu-store/tests/ashfall.rs::the_example_project_opens_and_indexes_every_node` opens `examples/Ashfall.wobu` through the production store. `docs/02-data-model.md` documents the same folder contract. |
| Style Guide and World Bible are unique roots, and the navigator pins them. | `wobu-store/tests/ashfall.rs::the_singletons_are_present_and_unique` checks the stored world; `Workspace.tsx` derives the pinned list from registry entries marked `singleton`. |
| A real species → culture → setting → character influence chain survives storage. | The Ashfall fixture contains Vashk, Ember Guild, Cinder Bay, and Kael Vantris. `wobu-store/tests/ashfall.rs::kael_carries_the_stack_the_prototype_described` proves their links resolve after opening the folder. |
| Notes survive close/reopen and remain ordinary Markdown. | `wobu-store/tests/ashfall.rs::every_file_survives_a_read_write_round_trip_byte_for_byte` and `::deleting_the_index_loses_nothing_from_the_folder` prove the text files, rather than the SQLite index, are authoritative. |
| External editors are supported without a private import/export format. | The fixture is hand-authored Markdown. The store and sync suites cover external edits, rescan, close/reopen, and index reconstruction; `docs/05-architecture.md` names Obsidian and `git pull` as supported writers. |
| M1 keyboard commands have an explicit executable map. | `useKeyboard.ts` owns workspace-wide navigation, while editing actions are owned by their surfaces. Workspace, undo, palette, and editor tests exercise individual routes; the release smoke pass below remains responsible for checking the complete documented map together. |
| Drag-to-reparent enforces the nesting rules and persists the move. | `drop.test.ts` covers valid, cross-kind, cyclic, read-only, and top-level drops. `wobu-core::node` validates cycles, and `wobu-store::Project::move_node` owns the guarded file move and index update. |
| Deleting a node with children has a stable, documented result. | `Project::delete_node` promotes children to the deleted node's parent and removes backlinks. `project/tests.rs::deleting_a_parent_promotes_its_children` and the undo tests pin that behavior. |

The remaining human smoke check is intentionally mundane: open the Ashfall
fixture in Wobu and Obsidian, make a short edit session, quit/reopen, and inspect
the resulting `git diff`. It validates the desktop and third-party packaging on
the current host; it is not a substitute for the contracts above and it should
be repeated for a release candidate.

## M6 — first generation loop

The canonical GitHub milestone is M6. The closed acceptance tracker retains its original
pre-sync-insertion title, [“M5 acceptance” #57](https://github.com/krazyjakee/wobu/issues/57).

| Acceptance claim | Repeatable evidence |
| --- | --- |
| A complete authored world resolves upstream-first into a character prompt. | The Ashfall store and influence fixture suites open the same Style Guide, World Bible, species, culture, setting, and character chain and pin stack order and compiled output. |
| Enhance receives stack context and stores structured output. | The LLM fake-provider and schema-agreement suites cover every kind; the app's enhance task records guarded updates and the review path handles re-enhance diffs. |
| ComfyUI and Gemini receive negotiated image requests. | Both adapters implement the same `ImageBackend` contract. Their wire/workflow tests pin model capability negotiation, reference routing, output decoding, and cancellation mapping without spending provider credits. |
| Attribution changes when a layer is muted. | `wobu-influence/tests/fragments.rs::a_layer_turned_all_the_way_down_keeps_its_fragments_for_attribution` proves a muted layer remains visible but contributes no prompt weight. PromptBox tests prove the rendered prompt and attribution spans use that compiler answer. |
| Excess references produce a per-role drop report. | The influence reference-budget tests and adapter workflow tests pin role/mechanism budgets and dropped-reference reporting. Generation snapshots retain the kept and dropped attribution. |
| A generation can replay from its immutable receipt. | `generate/replay.rs::replay_plan` reconstructs the provider request from the stored record, and its replay tests mutate or delete current world state while asserting the original prompt, negative prompt, seed, controls, and reference order are retained. The stored compiled prompts are authoritative; the influence snapshot preserves attribution and reference reconstruction rather than regenerating prompt prose. |
| Cancelling local generation stops provider work. | `wobu-jobs/tests/queue.rs` covers cooperative cancellation and forced abort. The ComfyUI transport races network/WebSocket waits with the cancellation token and calls the provider queue-delete or interrupt endpoint according to whether the graph reached the GPU. |

The provider smoke check still needs configured ComfyUI and Gemini accounts: run
one image through each adapter, deliberately overflow one reference role, mute
the culture layer, replay the receipt, and cancel a second ComfyUI job while the
sampler is active. Those observations are credential-, model-, and host-specific,
so release notes should record the versions used rather than pretending the
repository can prove them offline.
