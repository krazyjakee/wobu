# 07 — Projects on File Shares

A project is a directory ([02](02-data-model.md)), and that directory is expected to live on
an SMB/NFS share, a NAS, or a sync folder so a small team can work from it. This document
covers what that costs and how we pay it.

Wobu is **not** building real-time collaboration. The target is the realistic case: two or
three people who mostly work on different parts of the world, occasionally at the same time,
and who must never silently lose each other's work.

## The conflict surface is small by construction

Most of the folder cannot conflict, and that is a design choice rather than luck:

| What | Write pattern | Conflict risk |
| --- | --- | --- |
| `assets/**` | content-addressed, write-once | **none** — same bytes, same path |
| `generations/**` | ULID-named, write-once, never mutated | **none** |
| `project.json` | rarely written, small | low |
| `nodes/**/*.md` | edited continuously | **this is the only real surface** |

So all the machinery below exists to protect one class of file: node Markdown.

## Presence, not locking

On open, Wobu writes a heartbeat file and refreshes it every 20 seconds:

```
.wobu/sessions/<session-ulid>.json    { user, host, opened_at, heartbeat_at, editing: [node-ids] }
```

Sessions whose heartbeat is older than 60 seconds are treated as dead and reaped. On open,
Wobu reads the others and surfaces them: *"Nadia has this project open."* When a node is
being edited elsewhere, its row in the navigator gets a quiet presence dot and the editor
shows a passive banner.

This is **advisory**, deliberately. Hard locks over a network share strand files whenever
someone's laptop sleeps or the VPN drops, and the recovery UX is worse than the problem. We
warn, we never block.

## Writes are atomic, and check before they land

Every node write is:

1. Serialise to `.wobu/tmp/<ulid>.part` — on the **same filesystem**, so the rename is atomic.
   (Using the OS temp dir would silently degrade to a copy across devices.)
2. Compare the target's current `(mtime, size, content-hash)` against what we loaded.
3. If unchanged → `rename()` over the target. Done.
4. If changed → **conflict**.

## Conflicts are surfaced, never merged

We do not attempt a three-way merge of prose. The loser's version is written alongside:

```
nodes/character/kael-vantris.md
nodes/character/kael-vantris.conflict-jake-20260731T142211Z.md
```

and the UI raises a conflict card offering a side-by-side diff with *keep mine / keep theirs /
open both*. This is the Obsidian and Dropbox convention, it is predictable, and because the
files are Markdown a human — or git — can resolve it properly.

Enhance and Generate are treated the same way: they write to the node like any other edit, so
a long-running Enhance that finishes after someone else saved raises a conflict rather than
clobbering.

## File watching does not work over the network

`inotify`/`FSEvents` do **not** see writes made by other hosts on NFS or SMB. Relying on
`notify` alone means a collaborator's changes are invisible until restart.

So the store detects whether the project path is on a network mount and picks a strategy:

- **Local filesystem** → `notify` watcher, near-instant, debounced ~400 ms.
- **Network mount** → poll a directory listing every 5 seconds (idle: 15 s), comparing
  `(path, mtime, size)` against the index. Only files whose stamp changed are re-read.

Polling a listing is cheap; re-reading hundreds of small files is not. That asymmetry is the
whole reason the index exists.

## Performance: assume every read is slow

Reading several hundred small Markdown files over SMB is genuinely slow — this is the thing
most likely to make Wobu feel bad on a NAS. Mitigations:

- **The index absorbs it.** After first open, the workspace renders entirely from the local
  SQLite index. The folder is only touched for changed files.
- **Thumbnails live in the project folder**, not local cache. They are content-addressed and
  conflict-free, so the first person to import an image pays the cost of generating thumbs
  and everyone else gets them for free — which is exactly backwards from putting them in
  local app data.
- **Grids bind to thumbs only.** Full-resolution originals are fetched on demand, one at a
  time, when an image is opened.
- **Meshes are lazy.** A `.glb` is only pulled when the 3D tab is actually opened.
- Long share operations show progress and are cancellable. A stalled NAS must never present
  as a frozen app.

## Failure modes we handle explicitly

| Situation | Behaviour |
| --- | --- |
| Share unmounts mid-session | Detect on write failure → banner, retry with backoff, block writes, keep the UI readable from the index |
| Read-only share | Detected on open → open in read-only mode, disable Enhance/Generate, say so plainly |
| Two people generate for the same node | No conflict — separate ULID generation records, both appear |
| Sync client mangles a file | Index hash mismatch → surface the file as corrupt, don't overwrite it |
| Clock skew between machines | Heartbeats compare against *our own* clock via file mtime deltas, not absolute timestamps |

## What we are explicitly not doing in v1

Real-time co-editing, operational transforms, a server component, or per-node hard locks.
If a team outgrows advisory presence, the honest answer is git on the project folder — which
already works, because the format was chosen for it.
