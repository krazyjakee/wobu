# Harbor Watch: the missing light

An original handwritten offline regression fixture. The narrator reports a missing harbor lantern
as witnessed, told by Mara, or unverified quay rumour. Trust determines whether the player may lead
the search. Recording either response increases trust by five and pauses for `record_watch`.

`scene.yaml` and `state.yaml` are typed narrative source. The six canonical JSON scenario records in
`scenarios/` cover low/high trust × witnessed/told/rumour. Each expects a pending command, saves a
checkpoint, simulates failure, restores that checkpoint and returns `recorded = true` on success.
No dialogue is generated and no host action runs.

To inspect the fixture in Wobu, create an empty project and close it. Copy `state.yaml` to its
`narrative/state.yaml`, `scene.yaml` to
`narrative/scenes/00000000000000000000000001.yaml`, and the six `scenarios/*.json` files to its
`narrative/scenarios/` directory. Reopen it, then choose Narrative → Build… → Scenario tests → Run all.
For manual Preview, declare command signatures `{"record_watch":[]}`.

Run the fixture suite from the repository's `src-tauri` directory:

```sh
cargo test -p wobu-narrative-scenarios
```

The checked-in fixture is generated from explicitly handwritten source and expectations by
`cargo run -p wobu-narrative-scenarios --example harbor_watch_fixture -- /tmp/new-harbor-fixture`.
This writer refuses an existing destination. It never invokes the runner to derive expected values.
The tests additionally mutate a consequence, check the precise first divergence, restore it and
require the original expectation to pass again.
