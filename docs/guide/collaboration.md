# Sharing a project

Two ways to work with other people: everyone in one folder, or everyone with their own copy, catching
each other up machine to machine. Neither needs a server, an account, or anything we run for you.

> **Expectations first** Wobu is not real-time collaboration. There are no live cursors and no
> typing in the same page at once. It is built for the realistic case: two or three people who
> mostly work on different corners of a world, occasionally at the same time, and who must never
> silently lose each other's work.

## One folder, several people

Put the world on a shared drive, a NAS, or a synced folder, and anybody who can see it can open it.
Nothing about the world lives outside that folder. Everyone uses their own keys, on their own
computer, on the same world.

### Most of the folder cannot clash

That is on purpose, and it is why any of this works:

| What | How it is written | Chance of clashing |
| --- | --- | --- |
| Pictures | Filed by contents, written once | **None** — the same picture, the same place |
| Records of what you made | Written once, never touched again | **None** |
| The world's settings file | Rarely written, tiny | Low |
| Your notes and descriptions | Edited constantly | **The only real risk** |

So everything below exists to protect exactly one sort of file.

### It tells you, it does not lock you out

When you open a world, Wobu leaves a note in the folder saying you are here and refreshes it every
20 seconds. It reads everybody else's every 10 seconds and tells you about them: a line when you
open the world, a quiet dot on the row in the list, a count in the bottom bar, and a banner if you
open a page somebody else has open. Somebody whose note goes quiet for a minute is treated as gone.

That banner is worth reading once, because it explains the whole approach: nothing is locked,
nothing is switched off, and if you both save, the later save is put beside the file for you to sort
out rather than being written over the top of anything.

This is **a warning rather than a lock, on purpose**. Real locks over a shared drive strand files
every time somebody's laptop goes to sleep or a VPN drops, and digging yourself out is worse than
the problem. Wobu warns; it never blocks.

### Every save looks before it lands

1. Write to a temporary file **on the same drive**, so swapping it into place happens in one go.
2. Check the file being replaced — its date, its size, its contents — against what was loaded.
3. Unchanged: swap it in. Done.
4. Changed: **that is a clash**, handled below.

### You get both versions, never a merge

Wobu does not attempt to blend two people's writing together. The other version is written alongside
yours:

```
nodes/character/kael-vantris.md
nodes/character/kael-vantris.conflict-jake-20260731T142211Z.md
```

and a card appears above the editor showing the two side by side — headed **On disk now** and
*theirs* — with three choices: **Keep mine**, **Keep theirs**, or **Open both**, plus **Show in
folder**. Identical runs of lines are folded away, and if the two turn out to be word for word the
same, it says so.

There is deliberately no merge button. Both files are sitting in the folder and nothing is deleted
until you choose; the one you did not keep goes at that moment, and that is the only place in Wobu
a second version is ever deleted.

Enhance and Generate count as ordinary edits, so accepting an enhance after somebody else has saved
raises the same card rather than trampling their work.

### It notices changes differently on a shared drive

The usual way of watching for changes does **not** see writes made by other computers over a network
drive — relying on it alone would leave a colleague's work invisible until you restarted. So Wobu
works out what sort of drive it is on and picks its method: watching directly on your own disk,
checking every so often on a network one. The bottom bar tells you which, and how often.

### Speed on a shared drive

Reading several hundred small files over a network is genuinely slow, and it is the thing most
likely to make Wobu feel sluggish on a NAS. Four things soften it:

- **The search index takes the strain.** After the first open, the screen is drawn from Wobu's own
  local copy; the folder is only touched for files that actually changed.
- **Thumbnails live in the world folder**, not on your machine — so whoever adds a picture pays for
  making them once and everybody else gets them free.
- **Grids only load thumbnails.** Full-size pictures are fetched one at a time, when you ask.
- **3D shapes are fetched only when you open the 3D tab.**

Anything slow shows progress and can be cancelled. A stalled NAS must never look like a frozen app.
If typing feels heavy, raise the save delay in Settings → Editor.

## Copies that catch each other up

Open the world menu in the title bar and choose **Share this project…**. Wobu makes a ticket, shows
it to you and copies it. The other person chooses **Accept ticket…** on the opening screen, pastes
the whole ticket in, and picks where their copy should live. Wobu shows the transfer happening, lets
them cancel, and opens the copy when it is ready.

Pasting a ticket for a world that machine already has opens the copy it already has instead of
making a second one. If that copy has been deleted, or lives on a drive that is not plugged in, Wobu
asks where to put a fresh one — so deleting a half-finished copy and pasting the ticket again is a
way out rather than a dead end.

> **A ticket is a key to your world** Send it privately. Tickets do not expire, and you cannot take
> one back on its own. **Stop sharing…** cancels every ticket this installation ever made for this
> world and forgets the machines it was talking to — but it cannot delete copies people have already
> downloaded.

> **You both have to be running Wobu at once** Nothing is parked on a server in between. Somebody
> else's offline edits arrive the next time you are both running the app — the same as Syncthing or
> Resilio. The bottom bar says exactly that: edits arrive only while both of you have Wobu open.

### How the two copies work it out

No clever merging algorithms, and **no change to the files themselves** — they stay hand-editable
and other apps keep working. Wobu already knows a fingerprint for each page; syncing also remembers,
for each person, the last version you both agreed on. That gives a three-way comparison with nothing
new added to your files:

| Yours vs agreed | Theirs vs agreed | What happens |
| --- | --- | --- |
| Same | Changed | Take theirs |
| Changed | Same | Send yours, nothing to apply |
| Changed | Changed the same way | Agree, and move on |
| Changed | Changed differently | **Both versions kept**, exactly as above |

Pictures and records skip all of this: written once and filed by contents means a missing file is
only ever *fetched*, never merged.

### Two limits worth knowing up front

- **Deleting does not travel.** A file you have and they do not could mean "they deleted it" or "they
  never had it", and Wobu takes the reading that cannot destroy work. So deleting something on your
  copy does not delete it on theirs.
- **A ticket cannot be taken back on its own.** Once you have shared it, you have shared it.

## Read-only worlds

Whether Wobu can write to a folder is decided once, when the world opens, by trying it. If it
cannot, you get one banner saying so, a badge on the title bar and on the opening screen, and
everything that writes is switched off: making, renaming, moving, deleting and copying pages,
editing notes, saving, undo and redo, Enhance, Generate, and choosing services for the world.

The world stays entirely readable, and the website export still works — it only reads.

## When things go wrong

| What happened | What Wobu does |
| --- | --- |
| The shared drive vanishes mid-session | Notices, shows a banner, keeps trying, holds your saves — and keeps the whole app readable from its local copy. A held save goes through when the drive comes back. |
| The drive is read-only | Noticed when the world opens, as above. |
| Two people generate for the same page | No clash at all. Separate records, both results appear. |
| A sync app mangles a file | It is listed at the top of the world list with the actual error and a **Show in folder** button. It is never quietly skipped and never written over. |
| Your clocks disagree | Wobu compares how long ago things happened against your own clock, not absolute times. |

## If you outgrow this

Real-time co-editing, a server, and proper locking are all deliberately out of scope. If a team
outgrows the gentle version, the honest answer is **put the world folder in git** — which works
today, because the format was chosen with exactly that in mind.
