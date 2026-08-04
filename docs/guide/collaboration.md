# Sharing a project

Two ways to work together: several people on one folder, or several people holding copies that catch
each other up directly between machines. Neither involves a server, an account, or infrastructure
Wobu runs on your behalf.

> **Set expectations first** Wobu is not real-time collaboration. There are no live cursors and no
> co-editing. The target is the realistic case: two or three people who mostly work on different
> parts of the world, occasionally at the same time, and who must never silently lose each other's
> work.

## One folder, several people

Put the project on an SMB or NFS share, a NAS, or a sync folder, and anyone who can see the path can
open it. Nothing about the project lives outside that directory. They use their own keys, on their
own machine, against the same world.

### Most of the folder cannot conflict

That is a design choice rather than luck, and it is why this works at all:

| What | Write pattern | Conflict risk |
| --- | --- | --- |
| Assets | Content-addressed, write-once | **None** — same bytes, same path |
| Generations | ULID-named, never mutated | **None** |
| `project.json` | Rarely written, small | Low |
| Entity Markdown | Edited continuously | **The only real surface** |

So all the machinery below exists to protect exactly one class of file.

### Presence, not locking

On open, Wobu writes a heartbeat file into `.wobu/sessions/` and refreshes it every 20 seconds. It
reads everyone else's every 10 seconds and surfaces them: a line when you open the project, a quiet
dot on the navigator row, a count of who else is here in the status bar, and a banner in the editor
of an entity somebody else has open. A session whose heartbeat goes stale for a minute is treated as
dead.

The banner is worth reading once, because it states the whole model: nothing is locked, nothing is
switched off, and if you both save, the later save is parked beside the file as a conflict for you
to resolve rather than overwriting anything.

This is **advisory on purpose**. Hard locks over a network share strand files every time somebody's
laptop sleeps or a VPN drops, and the recovery experience is worse than the problem. Wobu warns; it
never blocks.

### Writes are atomic and check before they land

1. Serialise to a staging file **on the same filesystem**, so the rename is genuinely atomic rather
   than quietly degrading to a copy.
2. Compare the target's current state — timestamp, size and content hash — against what was loaded.
3. Unchanged: rename over the target. Done.
4. Changed: **conflict**, handled below.

### Conflicts are surfaced, never merged

Wobu does not attempt a three-way merge of prose. The losing version is written alongside the
winner:

```
nodes/character/kael-vantris.md
nodes/character/kael-vantris.conflict-jake-20260731T142211Z.md
```

and a conflict card appears above the editor with a side-by-side diff — headed **On disk now** and
*their* version — and three decisions: **Keep mine**, **Keep theirs**, or **Open both**, plus **Show
in folder** to open the folder. Identical runs of lines are collapsed, and if the two files turn out
to be line-for-line identical it says so.

There is no merge button, deliberately. Both files are on disk and nothing is deleted until you
choose; the one you do not keep is removed at that moment, and that is the only place in Wobu a
conflict sibling is deleted.

Enhance and Generate are treated as ordinary edits, so an enhance accepted after somebody else saved
raises a conflict rather than clobbering their work.

### Change detection adapts to the mount

File-watching APIs do **not** see writes made by other hosts over NFS or SMB — relying on them alone
would leave a collaborator's changes invisible until restart. So Wobu detects the mount type and
picks a strategy: a native watcher on a local filesystem, and polling on a network mount. The status
bar tells you which mode you are in, with the poll interval.

### Performance on a share

Reading several hundred small Markdown files over SMB is genuinely slow, and this is the thing most
likely to make Wobu feel bad on a NAS. Four mitigations:

- **The search index absorbs it.** After the first open, the workspace renders entirely from the
  local index; the folder is only touched for files that actually changed.
- **Thumbnails live in the project folder**, not local cache — so the first person to import an
  image pays to generate them and everyone else gets them free.
- **Grids bind to thumbnails only.** Full-resolution originals are fetched one at a time, on demand.
- **Meshes are lazy** — a `.glb` is pulled only when the 3D tab opens.

Long share operations show progress and can be cancelled. A stalled NAS must never present as a
frozen app. If the autosave feels heavy, raise the delay in Settings → Editor.

## Copies, synced peer-to-peer

Open the project menu in the title bar and choose **Share this project…**. Wobu creates a ticket,
displays it and copies it for you. The recipient chooses **Accept ticket…** in the launcher, pastes
the complete ticket, and picks where the local clone should live. Wobu shows transfer progress, lets
the recipient cancel, and opens the clone when it is ready.

> **A ticket is an access credential** Send it privately. Tickets do not expire and one ticket
> cannot be revoked by itself. **Stop sharing…** revokes every ticket this installation issued for
> this project and forgets sync history with its peers — but it does not delete copies collaborators
> have already downloaded.

> **Both peers must be online at the same time** Nothing is kept on an always-on server in between.
> A collaborator's offline edits land the next time you are both running Wobu — the same behaviour
> as Syncthing or Resilio. The status bar says so in as many words: peer edits arrive only while
> both people run Wobu.

### How the two copies catch up

No CRDTs, no vector clocks, and **no change to the on-disk format** — files stay hand-editable and
Obsidian keeps working. Wobu already stores a content hash per entity; sync additionally remembers,
per peer, the hash you last agreed on. That gives a three-way compare with no new state in the
Markdown:

| Yours vs base | Theirs vs base | Outcome |
| --- | --- | --- |
| Same | Changed | Fast-forward — take theirs |
| Changed | Same | Send yours, nothing to apply |
| Changed | Changed, identical bytes | Converged — just move the base |
| Changed | Changed, different bytes | **Conflict** — same sibling file as above |

Assets and generations skip all of this: content-addressed and write-once means a missing file is
only ever *fetched*, never merged.

### Two limits worth knowing up front

- **Deletes do not propagate.** There are no tombstones, so a file you have and a peer does not is
  ambiguous between "they deleted it" and "they never had it" — and the safe reading of an ambiguous
  absence is the one that cannot destroy work. Deleting an entity on your copy does not delete it on
  theirs.
- **Tickets cannot be revoked individually.** Once you have shared one, you have shared it.

## Read-only projects

Whether a folder is writable is decided once, when the project opens, by trying to write to it. If
it is not, Wobu raises one banner saying so, badges the title bar and the launcher card, and
switches off everything that writes: creating, renaming, moving, deleting and duplicating entities,
editing notes, autosave, undo and redo, Enhance, Generate, and the project's provider selection.

The world stays fully readable, and the static wiki export still works — it only reads.

## When things go wrong

| Situation | What Wobu does |
| --- | --- |
| Share unmounts mid-session | Detects it, shows a banner, retries with backoff, holds writes — and keeps the whole interface readable from the local search index. A held save resends when the share returns. |
| Read-only share | Detected on open, as above. |
| Two people generate for the same entity | No conflict at all. Separate ULID-named records; both results appear. |
| A sync client mangles a file | The file is listed at the top of the navigator with the parser's own error and a **Show in folder** button. It is never silently skipped and never overwritten. |
| Clocks differ between machines | Heartbeats compare timestamp deltas against your own clock, not absolute times. |

## And if you outgrow this

Real-time co-editing, operational transforms, a server component and per-entity hard locks are all
explicitly out of scope. If a team outgrows advisory presence, the honest answer is **git on the
project folder** — which already works, today, because the format was chosen for exactly that.
