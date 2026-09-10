# Narrative compiler and reference runtime, version 1

The `wobu-narrative-compiler` and `wobu-narrative-runtime` crates implement the pure Rust
foundation for #158 and #159. They do not perform generation, open files, use Tauri or access a
provider. Engine adapters and cross-language conformance are N5 work and are intentionally excluded.
Portable export packaging is tracked separately in #160.

## Compilation

`compile(scenes, schema, options)` takes complete scene sources, a validated `StateSchema`, the
complete known world entity set and a registry of host command argument domains. It returns
`CompileReport { graph, diagnostics }`. Any error suppresses the graph. Diagnostic records identify
the source scene and the existing typed source `Site`, with a severity, code and message.

Compilation checks source identities, entry conditions, destinations, participants, speakers,
conditions, effects, wording revisions and command registrations. IDs must be unique across the
whole input, including element kinds. Command arguments must match the registered arity and domains;
a variable argument's entire declared domain must fit the command argument domain. Integer assignment
between different ranges is allowed by the source type system and checked against the destination
range when executed.

The development profile warns about missing, empty, unapproved or stale text so structural work can
continue. A missing slot still produces a runtime `NoMatch` if executed. Release compilation rejects
all of these conditions. Approval is separate from locking, and a lock does not bypass freshness.
These checks use the lifecycle stored in source; dependency freshness recomputation and proof that
all reachable state configurations have text remain separate planned compiler passes.

Runtime IR contains declared typed state, registered command signatures, scene entry conditions, the
`setting` node a scene names, explicit beat graphs, dialogue strings and revisions, branches, effects,
stable string IDs, host-triggered supporting text assets with their triggers, conditions and selection
policies, and quests with their stages, transitions and objective wording.
Intents, summaries, world context, source descriptions, generation policy, review records, provenance
records and canvas layouts are omitted. The source map connects IDs to scene/beat/slot IDs; it does
not claim YAML byte locations. Conditions and consequences are copied only from authored structured
source. Dialogue strings are never interpreted as logic or commands.

Scenes, beats, state declarations, command registrations and source maps serialize in sorted map
order. Dialogue slots, variants, choices and outcomes preserve authored order. The first authored
beat is the entry point for a scene; reordering subsequent beats changes no destinations. Canonical
JSON contains no timestamps or machine paths. The graph's BLAKE3 hash is domain-separated and includes
the graph version, profile, state schema, text and all executable content. Renaming display titles or
moving canvas nodes does not change this hash. Identical compiled inputs produce identical bytes.
This is an internal Rust contract, not the N5 release package format.

## Selection and state

- `None` conditions and `Always` are true; `Never` is false. `All([])` is true and `Any([])` is false.
  Boolean expressions short-circuit in authored order. Equality works on matching bool/int/enum
  values; ordering is defined only for integers. Missing variables and mixed types are errors.
- Each dialogue slot selects the first matching variant in authored order, once, when entered.
  An unconditional variant is therefore a fallback only when placed after conditional variants.
  The selected variant ID is saved; changing host inputs while displaying a line does not replace it.
- After dialogue, all available choices are offered in authored order. If none is available, the
  first matching automatic outcome is taken. If neither exists, execution returns `NoMatch`.
  There is no implicit fallthrough to another beat and no invented ending.
- Version 1 has no random alternative-selection policy *for scene dialogue*. Variants remain
  first-match in authored order and are never sampled. First-match precedence resolves overlapping
  predicates; the compiler does not claim to prove overlap or coverage. The playthrough seed does
  affect host-triggered supporting text under its shuffled policy — see below and
  [supporting text](33-narrative-supporting-text.md).
- State uses booleans, bounded signed 64-bit integers and declared enum members. Arithmetic is
  checked and errors on both machine overflow and declared-range overflow; it never wraps or clamps.
- Standard `start` initializes narrative variables from defaults and requires every host variable
  explicitly. `start_with_state` additionally permits validated narrative overrides for a test
  scenario before scene entry is evaluated, without changing graph defaults or the graph hash.
  Unknown variables, wrong types and out-of-range values fail initialization.
- After initialization, only narrative effects write narrative variables. Host input updates and
  command results may write only host variables and are validated before anything is committed.

## Host-triggered supporting text

Barks, ambient exchanges, companion reactions, codex entries, quest summaries and journals (#167)
compile into a `texts` map beside the scenes. They have no destinations, no choices and no effects,
so they are delivered outside the scene cursor rather than through a yield.

`text_events()` lists every host event the graph answers. `deliver_text(event)` selects and records
one delivery: `Ok(None)` when nothing is eligible, `Err` when the content is broken, and otherwise
the whole entry at once — asset, kind, entry, ordered lines with speaker/wording/revision, and the
play count. Assets are considered in stable id order, so the older of two answering the same event
wins. A failed delivery is transactional like every other public action and leaves play counts
unchanged. Delivery never moves the scene cursor and never writes narrative state; a game that wants
a codex page to matter sets a host-owned variable itself.

Selection is authored per asset: `first` (never advances), `once` (each eligible entry once), `cycle`
(round robin) or `shuffle` (each eligible entry once per seeded round, never opening a round with the
entry that closed the last). The shuffle is SplitMix64 over a seed folded with FNV-1a, both written
out in the runtime so an engine adapter can reproduce the exact order. Its Fisher-Yates modulo is
slightly biased for lengths that do not divide 2^64, which is accepted for transcribability.

Snapshots carry a per-asset record of plays, round, position and the entry that closed the previous
round, so a reload resumes a bag rather than re-dealing it. `restore` refuses a record naming an
asset the graph does not contain, exactly as it refuses an invalid visit history, and migration may
add, transform or drop these records alongside visits. A graph and a snapshot for a project with no
supporting text serialize to exactly the bytes they did before this field existed.

## Quests and objectives

A quest (#207) compiles into a `quests` map beside the scenes and the texts: a declared stage list, an
initial stage, the transitions between them, and the player-facing objective wording for each stage.
A quest has no beats, no targets and no effects, so advancing one cannot move the story or write a
variable — it only records where in the story the player is.

After every write to state the runtime walks each quest forward. Transitions are tried in author
order and the first whose condition holds wins, which is the same first-match rule a beat's outcomes
follow. Advancement is bounded by the number of stages the quest declares, so a cycle of
unconditional transitions settles instead of spinning, and it never moves a quest backwards on its
own.

`quests()` gives the stage each quest is in. `objectives()` gives what the player should be doing, in
quest id order, with the wording's stable id so a host on a translated build looks it up in the string
table exactly as it looks up a line of dialogue. Reading either never advances anything. A stage whose
objective has not been written yields an empty string rather than no row, because "this quest is here
and nobody wrote what to do" is the honest answer — and the compiler has already said so.

Objective wording lives in `strings/en.json` with every other string, keyed by the same kind of
identity, so a locale pack is one list rather than two. A reachable stage with no objective is a
warning in Development and refuses a Release compile, exactly as a dialogue slot with no wording does.
A stage nothing leads to is exempt: requiring wording for a stage that cannot be entered would be
busywork with no symptom.

Snapshots carry the reached stage per quest rather than recomputing it on restore, because a quest
that has passed through a stage has passed through it, and re-deriving from current state would walk
a quest through a condition that has since stopped holding. `restore` refuses a cursor naming a quest
the graph does not contain or a stage it does not declare. A graph and a snapshot for a project with
no quests serialize to exactly the bytes they did before this field existed, and a package containing
quests declares the `quests` capability so a reader that cannot deliver objectives refuses to load
rather than shipping an empty quest log.

## Runner protocol and transaction boundaries

`Runtime::start(graph, scene_id, host_inputs, run_id, seed, step_limit)` runs to the first yield.
`current()` returns that same yield without advancing. `advance()` accepts a displayed line (and is
idempotent at End); `choose(id)` accepts only an available choice at a Choices yield.
`complete_command(token, result)` acknowledges a pending command. `update_host_inputs(inputs)` is a
validated host update at the current boundary. An update that leaves no valid current yield is
rejected. API results are `Line`, `Choices`, `GameCommand` and `End`, or deterministic typed errors.

A public action is transactional: any error leaves variables, cursor, visits, selection and command
acknowledgements unchanged. A transition first evaluates its entire ordered effect list against a
private state copy. Assignments and increments affect subsequent effects. Each command's arguments
capture values at that exact position in the list. If any later effect fails, no state or command is
published. If validation succeeds, all narrative writes commit together before the first command is
yielded. All commands from that transition then run in order before following its destination.
Host results do not change arguments already captured for later commands in the same transaction.

A command token contains the caller's playthrough `run_id`, the graph hash and a monotonic sequence.
Hosts must use a distinct run ID for independent playthroughs. The token is retained by snapshots.
The same successful acknowledgement with identical host inputs is idempotent, even after restore;
a different result for an acknowledged token is rejected. Failed/cancelled commands return an error
and remain pending with their existing token, allowing a deliberate retry or abandonment of the run.
They do not roll back narrative writes already committed before the command yield.

The host is responsible for deduplicating external side effects by token and persisting that record
alongside its game save. The runtime cannot undo audio, a spawned entity or a cutscene that the host
already started. In particular, a crash after performing a command but before saving its
acknowledgement can re-yield that token. A downstream entry/no-match/step error can also reject an
acknowledgement; the external operation must remain safe to acknowledge again with the same token.

Every automatic action consumes the caller-supplied step budget, which must be positive. The driver
uses an iterative loop, so even a large budget cannot grow the native call stack through graph
cycles. Reaching the limit returns `StepLimit` and rolls back the public action. Hosts should choose
a modest budget for interactive work; the budget counts internal transitions, not displayed lines.

## Saves and compatibility

`Snapshot` is versioned, serde-serializable data containing the graph hash and version, cursor,
selected variant, typed state, visits, supporting text repeat state, seed, pending command
queue/tokens, successful acknowledgements, run ID and step limit. `restore(graph, snapshot)` validates graph compatibility, required state,
variable domains, cursor, visits and pending command signatures before exposing a yield. Saves at
Line, Choices, each pending command and End resume at the same boundary without reapplying effects.
Snapshots are not a signed anti-tampering format; hosts own save integrity and size limits.

Different graph content or versions are rejected by ordinary `restore`. Content migration is an
explicit opt-in API: `Runtime::restore_with_migration(old_graph, new_graph, snapshot, hook)` validates
the original save against its original graph before calling the hook. The hook receives a `Migration`
plan with exact source/target graph hashes, state, cursor, selected variant and visit history. It may
add or transform state, map stable cursor identities and deliberately remove obsolete visit entries.
Changing either expected graph hash fails. The result then passes the same complete restore validation
against the new graph, including its state schema. Unsupported graph/snapshot protocol versions remain
rejected; this is a content migration hook, not an unchecked deserialization escape hatch.

Pending commands and successful acknowledgement records cannot be edited through the hook. Migration
preserves their original playthrough token namespace and captured arguments, so a previously performed
external operation does not acquire a new identity. Pending command signatures/destinations and saved
host results must still validate in the target graph; incompatible changes fail rather than replaying
side effects. Ordinary save/restore and repeated acknowledgements continue to work after migration.
The hook exists in the pure Rust API; desktop Preview checkpoints stay pinned to their compiled graph
and do not silently migrate when source is edited.

## Decision and effect traces

`Runtime::trace()` returns the most recent action's `ExecutionTrace`. Each record contains stable
scene, beat and applicable choice/outcome/slot/variant IDs. Condition records describe the exact
expression, result and path within a nested expression; comparison leaves include the actual input
values they read. Short-circuited subexpressions produce no record. Transition records identify the
route actually taken, separately from conditions that merely passed. Ordered effect records include
the authored effect and before/after values for the variables it reads or writes. Command result
records capture failure/cancellation/success, host state changes and repeated acknowledgement status.

A failed transaction leaves the runtime snapshot unchanged and marks its trace `committed: false`
with the error. Any tentative effect values in that trace were rolled back. Traces are observational:
reading `current()` does not append decisions or replay effects. They are returned separately from
snapshots, so restoring a checkpoint starts a new trace boundary. At most 2,048 records are retained
per action; `omitted` explicitly counts later records and the UI labels the trace as incomplete.
Effect records contain only relevant variables to avoid duplicating the entire world per effect.
The desktop keeps the latest 100 action traces, with scrollable history and links into Script.

While paused at a host command, Preview offers success, failure and cancellation controls. Success
may return typed host-owned values using the paused graph's schema, even if saved source changes
meanwhile. Rust validates output ownership, domains and transport integer limits. Failure and
cancellation retain the pending command and its token, and display the error and rollback trace for
an explicit retry. No Preview result performs a host game action or writes canonical project state.

## Desktop Preview boundary

Desktop compilation registers only Character world nodes as valid participants, matching the cast
and speaker selectors. Other existing world nodes, such as Props or Style guides, cannot become
speakers by placing their IDs into scene source.

The desktop bridge uses JSON numbers in a JavaScript webview. It therefore rejects graph integers,
declared bounds, scenario values and snapshots outside ±9,007,199,254,740,991 before crossing that
boundary. Returned frames are checked too: an exact incoming visit or command counter must not
silently round after execution increments it. This is a desktop transport constraint; the pure Rust
compiler/runtime continue supporting signed 64-bit state. A portable encoding for wider values is
part of the deferred engine/package contract, not an implicit promise of this bridge.

## Verification

Rust integration fixtures cover canonical serialization, stripped context, source diagnostics,
release text gates, world/command references, all comparison operators, conditional precedence,
ordered effects and captured command arguments, host ownership, invalid actions, arithmetic failure,
entry conditions and cross-scene transitions, no-match, bounded loops, scenario overrides and save
roundtrips at every yield. They also exercise command failure/cancellation, duplicate acknowledgements,
invalid host results and incompatible/tampered snapshot fields. These are reference Rust tests;
N5 engine fixtures and native-engine validation remain excluded. The separate
[native Wobu walkthrough](21-native-narrative-preview.md) records desktop validation and its evidence.
