# Harbor Voices: supporting text for the missing light

Six original handwritten supporting text assets — one of each kind — set beside the
[Harbor Watch](../harbor-watch/README.md) scene fixture and sharing its declared variables.

| File | Kind | Trigger | Selection |
| --- | --- | --- | --- |
| `texts/seawall-keeper-bark.yaml` | Bark | `player_passes_seawall` | Shuffle |
| `texts/quay-talk-ambient.yaml` | Ambient exchange | `quay_idle` | Shuffle |
| `texts/mara-logbook-reaction.yaml` | Companion reaction | `logbook_opened` | Shuffle |
| `texts/harbour-watch-codex.yaml` | Codex entry | `codex_opened` | First |
| `texts/missing-light-quest-summary.yaml` | Quest summary | `quest_log_opened` | First |
| `texts/watch-nights-journal.yaml` | Journal | `night_ends` | Once |

`state.yaml` is a copy of the Harbor Watch declarations, so the conditions in these assets —
`recorded`, `knowledge` and `trust` — type-check on their own. Nothing here is generated: every
line is written by hand, every wording carries the revision it hashes to, and no host action runs.

## What each file demonstrates

- **Bark.** Four one-line entries, two of them conditioned on whether the failure has been
  recorded. Under `shuffle` the runtime deals each eligible entry once per round, in an order
  derived from the playthrough seed, and never opens a round with the line that closed the last
  one. Reloading a save resumes the same round rather than re-dealing it.
- **Ambient exchange.** Two versions of the same overheard conversation, each three lines long
  and alternating between two speakers. Each line is its own slot with its own identity, so the
  two voices are separately reviewable, translatable and recordable.
- **Companion reaction.** One line per trust band. The entry conditions decide which the player
  hears; there is no fallback entry, so below the threshold the companion has one thing to say
  and above it another.
- **Codex entry, quest summary and journal.** Standalone prose. They have no cast, no character
  voice and — because the model has nowhere to put one — no player choice. The quest summary
  uses `first`, so reopening the log with unchanged state shows the same words; the journal uses
  `once`, so it hands over one page per night and then has nothing more.

## Using it

From the repository's `src-tauri/` directory, create a fresh complete example:

```sh
cargo run -p wobu-store --example harbor_voices_fixture -- /tmp/harbor-voices-example
```

Open the printed `.wobu` path, then choose **Narrative → Text library**. The setup writes the three
real character records with their narrative voices, the linked **The missing light** quest,
declared variables and all six original assets using the normal guarded store. It refuses to
overwrite an existing project. Copying only the YAML text files into an empty project is
insufficient: their stable speaker and quest references must also exist.

Development export works immediately. Review and approve the exact wording/context before Release
export. For Context/Generate, use a complete scenario; for example `trust = 70`,
`knowledge = witnessed`, `recorded = true` selects the first entries of all six templates. Other
entries intentionally require different conditions. A host binds the six trigger names and calls
`Runtime::start_text` followed by `deliver_text`; no scene or player choice is needed.

`cargo test -p wobu-store --test harbor_voices` builds this same project with real nodes and
source links and checks ready frozen context for all six types.

These files are held to the format by `cargo test -p wobu-narrative --test harbor_voices_example`,
which parses every one of them, checks each against the declared state, requires every wording to
still hash to its recorded revision, and requires a read-then-write round trip to be byte identical.
