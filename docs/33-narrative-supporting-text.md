# Supporting text: barks, ambient exchanges, reactions, codex, quest summaries and journals

Everything a game says outside a scene. **Narrative → Text library** authors six kinds through the
same model, the same wording identities and the same export path as scene dialogue. This is N3
issue #167. It is planned work landing in stages; the limits at the end of this page say which
stages are not here yet.

## The model

One document per asset, at `narrative/texts/<slug>.yaml`, with `schema_version: 1`. An asset has a
kind, a name, a host trigger, optional source links, a cast, a selection policy and an ordered list
of entries. An entry has an optional condition and one or more lines. **A line is a dialogue slot**
— the same type a beat holds — so a bark carries the same stable slot and variant identities, the
same wording revision, the same provenance and the same generation policy, review state and
freshness as a line inside a scene.

There is no choice, outcome or destination anywhere in the model, and no field one could be put in.
US-09 requires standalone prose to exist without inventing a player choice to hang it on; the
guarantee is structural rather than a validator somebody could relax.

| Kind | Voice | One delivery is | Default selection |
| --- | --- | --- | --- |
| Bark | Cast, player or narrator | One line | Shuffle |
| Ambient exchange | Cast, player or narrator | An ordered sequence, expected to alternate speakers | Shuffle |
| Companion reaction | Cast, player or narrator | One line | Shuffle |
| Codex entry | Narrator or player only | An ordered passage | First |
| Quest summary | Narrator or player only | An ordered passage | First |
| Journal entry | Narrator or player only | An ordered passage | First |

An entity that speaks has to be in the asset's cast, exactly as a scene speaker has to be a
participant: otherwise a generation request has no voice to work from and an export has no entity to
attribute a recording to. Standalone prose has no cast at all — prose attributed to a character is
that character speaking, which is a different asset with different review and recording
consequences.

## Selection and repeat state

Selection is authored, not left to the host, because two hosts choosing differently would make the
same project play differently and the deterministic-content promise requires the engine adapter to
have no policy of its own.

- **First** — always the first eligible entry, and the play position never advances. Prose whose
  wording is a function of state must give the same answer to the same state every time it is read.
- **Once** — each eligible entry once, in author order, then nothing more.
- **Cycle** — round robin in author order, wrapping.
- **Shuffle** — a bag: each eligible entry once per round, in an order derived from the playthrough
  seed, the asset identity and the round number, never opening a round with the entry that closed
  the previous one.

Entry order therefore matters, unlike beat order inside a scene. Reordering entries preserves every
identity but does change which entry `first` and `once` reach.

The shuffle uses SplitMix64 over a seed folded with FNV-1a, both written out in full in
`wobu-narrative-runtime/src/text.rs` rather than taken from a crate, because an engine adapter in
another language has to reproduce the exact sequence. The modulo in the Fisher-Yates step is very
slightly biased for list lengths that do not divide 2^64; that is accepted deliberately, because
rejection sampling would be harder to transcribe correctly than the bias is worth.

This is the first place the playthrough seed changes what a player sees. Scene dialogue is
unaffected: variants remain first-match in author order and are never sampled.

## Host triggers and delivery

`Runtime::text_events()` lists every event a compiled graph answers, so a host can bind its triggers
once at load. `Runtime::deliver_text(event)` asks what to play and records that it played.

- `Ok(None)` means nothing eligible, which is the ordinary answer for most events most of the time
  and is deliberately not an error.
- `Err` means the content is broken — a line with no applicable wording, a condition over state the
  save does not contain — and, like every public action, leaves the runtime exactly as it was.
- Assets are considered in stable id order. Because narrative ids are ULIDs, that is creation order,
  so when two assets answer the same event the older one wins. This is arbitrary but fixed; an
  author who cares should condition them apart.

Delivery returns the whole entry at once — asset, kind, entry, ordered lines with speaker, wording
and revision, and the play count including this delivery. Supporting text has no branches and
nothing to wait for, so handing lines over one `advance()` at a time would create a second cursor
that a save would have to keep consistent with the first.

Two things this path deliberately cannot do. It cannot touch the scene cursor, so a bark fired
during a conversation cannot advance, rewind or reselect the line the player is reading. And it
cannot write narrative state: there are no effects on a text asset and nowhere to put one. A game
that wants reading a codex page to matter raises its own flag through a host-owned variable, which
is already how the host tells the narrative anything.

`Snapshot` carries a per-asset record of plays, round, position and the entry that closed the
previous round. It is saved rather than derived, because every derivation available at restore time
is wrong: the eligible set depends on state that has since changed, and counting from the visit log
would make a bark repeat itself after a reload. A save naming an asset the graph does not contain is
refused by `restore`, exactly as an invalid visit history is. Content migration may add, transform
or drop these records alongside visits.

A project with no supporting text serializes a snapshot and a graph to exactly the bytes it did
before this field existed, so existing saves, packages and scenario tapes keep validating. Adding a
bark does move the graph hash, which is correct: the game now says something it did not say before.

## Authoring, review and export

Manual authoring, generation policy, approval evidence, freshness and localisation all use the same
mechanisms as scene dialogue, because they operate on the same `Text` values.

- **Policy.** An asset carries its own `policy` alongside each line's, so locking an asset stops any
  future job touching it without walking every line first, and holds for a caller that never went
  near the UI.
- **Approval.** `TextTarget`, `TextBinding` and `TextApprovalEvidence` mirror their scene
  counterparts and share one definition of what an approval proves — the same speaker, the same
  wording revision, a stored revision that still describes the stored words, and a context revision
  matching the current one. Release compilation refuses supporting text that no reviewer approved,
  whose approval names other wording, or whose context has moved. Writable Approved flags in a file
  cannot authorise a release on their own.
- **Localisation.** Packaged supporting text goes into the same `strings/en.json` table keyed by the
  same variant identity and carrying the same wording revision. A translator working from that file
  cannot tell, and should not need to tell, whether a line is spoken in a scene or shouted at a gate.
- **Export.** A package whose graph contains supporting text declares the required capability
  `supporting_text: 1`. A reader written against version 1 without it would otherwise load the
  package, ignore the `texts` map it does not understand, and ship a game that never says a single
  bark — a failure with no symptom. Packages with no supporting text declare exactly the
  capabilities they always did, and their identity is unchanged.
- **Sync.** A peer cannot replace, weaken or remove hand-written or locked supporting wording, and
  cannot offer approved text whose stored revision no longer describes it. These are the scene rules
  applied through one shared implementation rather than a second, laxer copy.

## Diagnostics

`TextAsset::diagnostics` answers what can be settled by reading one asset against the declared
state, keyed to the element responsible so a message can be turned into a selection. Alongside the
shared problems — undeclared variables, missing text, a revision that no longer describes its
wording, a duplicated id — supporting text adds four:

| Code | Meaning |
| --- | --- |
| `no_text_entries` | The asset's trigger has nothing to deliver. |
| `prose_has_cast` | Standalone prose has participants; remove them. |
| `voice_not_allowed` | A character is voicing standalone prose. |
| `wrong_line_count` | A bark or reaction entry holds more than one line. |

Everything except missing text is an error at every profile, because none of it has a defensible
runtime meaning. Missing text is a task in Development and a blocker in Release, exactly as it is
inside a scene.

## The Text library

**Narrative → Text library** lists every asset with its kind, offers the six templates, and edits
one document at a time: name, trigger, selection policy, brief, asset lock, entries and wording.
The kind shapes the controls — a codex form does not offer a cast picker, a bark form does not offer
a second line — but the enforcement is the backend's, whose diagnostics the pane renders in the
backend's own words. Saving is a whole-document guarded write with the stamp the document was read
at, so a stale tab cannot overwrite a colleague's newer file, and a losing write is parked as a
conflict sibling for a human. Changed wording is re-sealed through `narrative_text_written`; nothing
on the webview side ever invents a revision.

## Example

[`examples/narrative/harbor-voices/`](../examples/narrative/harbor-voices/README.md) contains one
original handwritten asset of each kind, sharing the Harbor Watch declarations. A Rust test parses
every one of them, checks each against the declared state, requires every wording to still hash to
its recorded revision, and requires a read-then-write round trip to be byte identical.

## Verification and limits

Rust fixtures compile and play all six kinds together with a scene: ambient multi-speaker delivery,
bark bag exhaustion and the no-immediate-repeat rule, seed determinism, mid-bag save and restore,
`once` exhaustion, `first` stability, the refusal to write state, rollback on a line with no
applicable wording, restore refusing a save that names a missing asset, and the release gates for
unapproved and rewritten wording. Package tests cover string externalisation, the capability
declaration and a manifest that disagrees with its payload. Command tests cover creation, slugging,
the stamp guard, catalog-authoritative paths, deletion and the source fingerprint. React tests mock
IPC; they establish the displayed controls and the exact command arguments, not native Tauri
rendering.

Not done in this checkpoint, and not implied by it:

- **Generation.** The frozen generation request, its context and its proposal record all name a
  scene slot. Supporting text is not yet reachable from **Generate…**, so no provider call has been
  made against one and no context-specific prompt has been validated live. The policy, lock and
  approval machinery it will use is in place and tested.
- **Review queue.** The paged Review surface is scene-keyed and does not list supporting text.
  Approval evidence for supporting text is defined and enforced by the compiler, but nothing yet
  produces it from canonical editorial history, so a **Release** export of a project containing
  supporting text is blocked until that lands. Development export works.
- **Library search.** Supporting text is not in the SQLite projection, so the Text library reads the
  folder and offers no search, filters or paging.
- **Screenshots.** No UI evidence is recorded for this checkpoint.
