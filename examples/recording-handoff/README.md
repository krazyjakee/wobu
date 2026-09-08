# Recording handoff fixture

`harbor.wav` contains the original sentence “The harbor is quiet. Keep the lantern burning.”
It was prepared offline with the installed Flite KAL synthetic voice (8 kHz, mono, PCM16),
using its [public synthesis API](https://github.com/festvox/flite/blob/master/include/flite.h).
This is prepared test speech, not a live provider or engine-adapter validation. Wobu does not
bundle or invoke Flite. The timing cues below are hand-authored demonstrations, not forced alignment.

Create a new native project and external recording manifests/files:

```sh
mkdir -p /tmp/wobu-recording-example
cd src-tauri
cargo run -p wobu-store --example recording_fixture -- /tmp/wobu-recording-example
```

Open `recording-harbor.wobu` in Wobu. In Review → Recording, select `recording.csv` and the
parent `/tmp/wobu-recording-example` as the prepared files directory. Preview, then import.
The source is approved and locked; Release requires its English recording and timing sidecar.
Audition the imported take and observe playback milliseconds and the active word/viseme cues.
`partial.json` adds an unknown ID to prove eligible-row import without losing diagnostics.

Before import, Release is blocked. After import it succeeds. Unlocking or editing the same
source ID retains the old take and makes it out of date. Recording policy can explicitly allow
text-only fallback; stale audio is never rebound. See [the recording contract](../../docs/39-narrative-media.md).
