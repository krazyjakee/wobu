# Exit policy

What Wobu promises when it stops, on every path it can be stopped by.

The window is frameless (`decorations: false`), so the titlebar's own controls in
`src/components/WindowControls.tsx` are ordinary buttons rather than native chrome. There is
therefore no such thing as "the OS close path" and "the in-app close path" as separate things to
reason about: the button calls `close()`, which raises the same `CloseRequested` that Alt+F4, a
window-manager keybinding and a dock quit raise. One gate covers all of them.

## The promise

1. **No pending user edit is lost on a normal close.** Every close blocks until the autosave
   debounce has been cancelled, its patch sent, and the backend has answered.
2. **Running jobs are cancelled cleanly rather than orphaned.** Cancellation runs the adapter's own
   stop path, so a provider is told, ComfyUI gets its `/interrupt`, and what was billed is reported.
3. **Any close that would discard work asks first.** Unsaved text and unfinished jobs are both
   questions, never silent decisions.
4. **A kill Wobu cannot intercept still loses at most one debounce interval** — 500 ms by default,
   3 s at the slowest setting — and never corrupts a project.

Everything below is either how one of those is kept, or a stated exception to it.

## The gate

Two halves, in this order. The renderer's half is the only one that can reach a pending keystroke;
the process's half is the only one that runs when the renderer is not listening.

**Renderer — `src/hooks/useSafeWindowClose.ts` → `src/lib/projectClose.ts`.** The first
`CloseRequested` is refused outright. Then:

1. `editorWrites.flushAll()` — every mounted autosave hook cancels its debounce, sends its patch and
   waits for the answer. A write that fails leaves the workspace mounted with a sticky banner, so
   the text is still on screen and still recoverable.
2. `job_list` — anything queued, running or retrying refuses the close a second time, with a banner
   naming the jobs and a **Stop them and quit** action. Dismissing it means staying. This step is
   deliberately *after* the flush (so a user who decides to stay has already had their typing saved)
   and *before* `project_close` (so deciding to stay does not dump them in the launcher).
3. `job_cancel` for each of them, once the user has agreed.
4. `project_close`, then `destroy()` — which is the only call that bypasses the gate, and is only
   ever reached on the far side of it.

The gate runs whether or not a project is open, because the job queue is per installation and
outlives the project it was started for. Quitting from the launcher can still destroy a generation.

**Process — `src-tauri/src/shutdown.rs`.** `wind_down` is idempotent and is what every exit path in
the process runs, in one order:

1. `Queue::close()` — cancel every unfinished job, and refuse to admit new ones. A job submitted
   after this is born cancelled, so a command racing the teardown cannot start a paid call on the
   way out.
2. `Queue::quiesce(3s)` — wait for them, briefly. Each job is already bounded by the queue's
   two-second `cancel_grace`, so reaching this budget means something below it is wedged, and the
   answer to that is to leave anyway. A quit that hangs is worse than a job that is cut off.
3. `AppState::close()` — drop the presence entry (collaborators see us leave now rather than up to
   60 s later), stop the watcher and reconnect threads, close the SQLite index.
4. `SyncState::stop()` — wind the iroh endpoint down with its own 5 s budget, so peers are told
   rather than finding a severed socket.

Jobs come before the project because a generation writes its result by opening the folder *by
path*, not through `AppState`: closing the project first would not have stopped it, it would only
have removed the watcher that notices what it wrote. Sync comes last because closing the project is
where its mutex is contended.

## Every path

| Path | What is in flight | Policy | Safe now |
| --- | --- | --- | --- |
| Titlebar close button (frameless) | Autosave debounce; jobs; sync round | Full gate. The button raises `CloseRequested` like any other close; it must never call `destroy()`. Guarded by `src/components/WindowControls.test.tsx`. | Yes |
| OS close — Alt+F4, WM button, dock quit, `Cmd+Q` | Same | Same `CloseRequested`, same gate. | Yes |
| Project close (back to the launcher) | Autosave debounce; jobs | Block-and-flush the writes, then `project_close`. Jobs are **not** cancelled and **not** warned about: they are per installation and a generation records its result by path, so it lands in the folder whether or not the world is still open. | Yes |
| Quit while the share is unreachable | Held edits that cannot be written anywhere | Refused, twice over. `flushAll` cannot succeed against an unreachable folder, so the editor banner holds the window; `lib.rs` also refuses `CloseRequested` and emits `share:quit-blocked`, whose banner offers **Quit anyway** → `force_quit`. | Yes, with a deliberate escape hatch |
| Quit with jobs in flight | Paid generations, ComfyUI runs, enhance calls | Warn, then on confirmation cancel each one and wait up to 3 s. Not awaited to completion: a generation takes minutes and an app that sat on a quit for one would look hung. | Yes |
| `SIGTERM` / `SIGINT` / `SIGHUP` — logout, shutdown, `systemctl stop`, Ctrl-C | Everything above, and no renderer round trip available | `shutdown.rs` catches all three on Unix and runs `wind_down` before exiting. The renderer is *not* consulted, so up to one debounce interval of typing is lost. Everything already written is durable. | Partly — see [Accepted losses](#accepted-losses) |
| Windows logout / shutdown | Same | `WM_CLOSE` arrives as an ordinary `CloseRequested`, so the full gate including the renderer's half runs. | Yes |
| `SIGKILL`, power loss, OOM kill, GPU driver reset | Same | Nothing runs. Recovery is structural, not procedural — see below. | Partly |
| Webview crash | The renderer's half of the gate | The process survives; `lib.rs`'s offline refusal and `wind_down` still run. Whatever was in the debounce is gone with the renderer. | Partly |
| Rust panic in a job | That job | Caught at the queue's task boundary, reported as `internal`, never retried. Other jobs and the app are unaffected. | Yes |

## Durability, per store

What survives a kill has nothing to do with the exit path and everything to do with how each thing
is written.

- **Node Markdown** — `wobu-store`'s `atomic.rs`. Staged in `.wobu/tmp` on the same filesystem,
  `sync_all`'d, then renamed over the target under a compare-and-swap against the stamp we last
  read. An interrupted write leaves either the old file or the new one, never a partial one, and a
  losing write is parked as a `.conflict-*.md` sibling rather than overwriting anyone.
- **Generation JSON and imported assets** — write-once, published with `hard_link`, which cannot
  replace a name another writer has claimed. An interrupted publish leaves a `.part` file in
  `.wobu/tmp` that the next write cleans up.
- **Project metadata** — `replace_metadata`, with a Windows recovery link that `recover_replace`
  restores if the process dies in the remove/link gap.
- **The SQLite index** — WAL, `synchronous=NORMAL`, on local app data rather than the share. A
  crash costs at most the last transaction, and the index is a *cache*: the folder is canonical and
  `index_rebuild` reconstructs it. This is why a hard kill is survivable at all.
- **The presence session file** — removed on `AppState::close()`. A kill leaves it behind, and it is
  reaped by mtime after 60 s, so a stale entry is bounded and self-healing.
- **Recents, keys, machine settings** — outside the project, written whole, and not user work.

## Accepted losses

Stated so they are decisions rather than surprises.

- **Up to one debounce interval on a signal or a kill.** 500 ms by default; the setting's ceiling is
  3 s. Closing the interval entirely would mean writing on every keystroke — one guarded
  `write`+`rename` per character, over SMB — which trades a bounded loss nobody has hit for a
  latency problem everybody would.
- **A cancelled paid job is not refunded.** Cancelling stops the request and reports what was
  billed, so the user is told rather than left to assume; it cannot un-bill it. The warning says so
  in those words.
- **An enhance result that has come back but not been accepted** is dropped on project close and on
  quit — `Pending` is in-memory and per installation. It is a re-runnable text call, and holding
  unaccepted suggestions across sessions would mean showing the user a proposal about a world they
  may have since changed.
- **A sync round in flight is cut.** Its writes are individually atomic and its base only advances
  on acknowledgement, so the cost is a node re-offered on the next round, not a merge that half
  happened.
- **A `SIGKILL` reaches nothing.** By definition. The storage design above is the whole answer.

## Where the tests are

- `src/hooks/closeCoordination.test.tsx` — the renderer's gate end to end: the debounce flushed
  before `project_close`, a failed save keeping the workspace alive and retrying the retained patch,
  the jobs warning refusing the first quit, **Stop them and quit** cancelling every unfinished job
  before the window goes, finished jobs not holding a quit, the launcher path with no project open,
  and a backend that cannot answer `job_list` still letting the window close.
- `src/components/WindowControls.test.tsx` — the frameless close button asks to close rather than
  closing, so the gate cannot be bypassed by the one control most likely to be "simplified".
- `src-tauri/crates/wobu-jobs/tests/queue.rs` — `close` + `quiesce` stopping queued and running work
  and reporting each as cancelled, an adapter that ignores its token still being gone when the queue
  quiesces, `quiesce` giving up rather than hanging, and work submitted after `close` never running.
- `src-tauri/src/shutdown.rs` — the teardown order, and that a second wind-down is free.
