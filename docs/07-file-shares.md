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
| `narrative/layout/**` | edited continuously, merged per node | **none** — see below |
| `nodes/**/*.md` | edited continuously | **this is the only real surface** |
| `narrative/scenes/*.yaml`, `narrative/state.yaml` | edited continuously | same surface, same rules |

So all the machinery below exists to protect two classes of file: node Markdown and narrative
source. They are treated identically — guarded write, conflict sibling, never merged.

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

## Canvas layout is the one thing we do merge

`narrative/layout/**` holds where the boxes are on the Flow canvas (#185): node positions,
collapsed groups, layout mode, pinned notes. It is merged **per node id, last write wins**,
and a losing write is never parked as a conflict sibling. That is a deliberate exception to
everything above, and it is worth being explicit about why:

- Two writers dragging two different boxes is not a semantic disagreement. There is no version
  of the arrangement that is "theirs" as opposed to "ours" — there is one canvas with two boxes
  on it, and both moves are correct.
- A diff card over a coordinate is a diff card nobody will read. The conflict card works
  *because* it is rare and always about words somebody wrote. Raising one every time two people
  had a scene open would train people to dismiss the card without looking, and the next one
  would be a paragraph.
- Layout must never be able to block a source save, and the cleanest guarantee of that is a
  separate file that resolves itself. Saving a scene does not touch it at all.

What the merge cannot do, stated so it is a decision rather than a surprise:

- **It resolves by wall clock.** Each entry carries the `updatedAt` of whoever set it, so two
  machines with skewed clocks resolve in favour of the fast one, not the recent one. There is no
  vector clock here, because acquiring one would mean a per-peer table for a file whose
  worst-case loss is a rectangle in the wrong place.
- **Exact ties break by content hash**, which has no claim to being right — only to being
  deterministic. Without it two machines merging the same pair of files in opposite directions
  reach different answers, the folders never converge, and the file ping-pongs forever.
- **Deletion loses to movement.** The merge is a union, so an entry one side removed and the
  other kept comes back; it is dropped again the next time the layout is read against the
  source, which is where deleted ids are collected anyway.
- **Groups and annotations merge whole.** Two people adding different beats to the same group
  keeps only the later edit's membership.
- **The write is not a compare-and-swap.** Neither POSIX nor SMB has a rename that fails when
  the target moved. The write re-reads and re-merges up to three times and then lands
  regardless, which narrows the lost-update window to the gap between a `stat` and a `rename`.
  Inside that window one collaborator's simultaneous drag can be dropped.

Nothing in that list can cost anybody a sentence they wrote. Everything a person types stays in
the source document, where the never-merge rule applies in full.

## File watching does not work over the network

`inotify`/`FSEvents` do **not** see writes made by other hosts on NFS or SMB. Relying on
`notify` alone means a collaborator's changes are invisible until restart.

So the store detects whether the project path is on a network mount and picks a strategy:

- **Local filesystem** → `notify` watcher, near-instant, debounced ~400 ms. It watches
  `nodes/`, `narrative/` and the project root itself. The root is watched non-recursively and
  for one reason: a project with no narrative has no `narrative/` directory to watch, and
  creating one at open would change a folder we promised to open unchanged — so the tree is
  attached the moment it appears, whether that is this machine's first scene, a peer's sync
  round or a `git pull`.
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
- **Meshes are lazy.** The closed 3D tab asks for nothing. Opening it lists only fixed-size GLB
  headers, then streams and hashes the selected file once into a disposable machine-local cache;
  three.js reads that local copy, so one view does not pull the same large GLB over SMB twice.
- **LoRA blobs use the large-blob path.** Sync accepts only the exact
  `assets/loras/<prefix>/<hash>.safetensors` path for the advertised hash and allows up to 30
  minutes for these project-owned weights, while ordinary blobs keep the shorter timeout.
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
