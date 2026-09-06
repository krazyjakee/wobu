# Keyboard, settings and files

Shortcuts, every setting, what is in the folder, what to try when something is wrong, and a
glossary.

## Keyboard

Every key Wobu listens for is in one list, and **all of them are yours to change**. Settings →
Keyboard shows them, lets you press a new combination onto any row, shows what it shipped as beside
anything you have changed, and offers **Reset every shortcut**. The same list, read-only, is one
keypress away from anywhere — the [shortcuts reference](wobu:shortcuts).

So treat the table below as *what this version came with*, not as the truth about your copy. It is
also why tooltips throughout the app tell you the keys instead of this page doing it: a tooltip
reads the real list, so it follows you when you change something.

`Mod` is Command on macOS and Control everywhere else. Your shortcuts are stored once, not per
computer; only the way they are written on screen changes.

| Default | Command |
| --- | --- |
| `Mod+K` | Command palette |
| `Mod+/` | Keyboard shortcuts |
| `Mod+F` | Filter the navigator |
| `Mod+Shift+L` | Library |
| `Mod+\` | Forge, and back |
| `Mod+Shift+A` | Assets |
| `Mod+,` | Settings |
| `[` | Toggle the navigator |
| `]` | Toggle the inspector |
| `Mod+Shift+C` | Collapse or expand everything |
| `Mod+1` … `Mod+5` | Notes, References, Concepts, 3D, Relations |
| `Mod+N` | New entity |
| `Mod+Z`, `Mod+Shift+Z` | Undo, redo |
| `Mod+E` | Enhance the open entity |
| `Mod+Enter` | Generate |

Three things cannot be changed, because an app where they could would be broken rather than
flexible: **Escape** closes whatever is on top, **Tab** and **Shift+Tab** move around inside a
dialog and stay inside it, and the **arrow keys and Enter** choose things in a list or a menu.

Most shortcuts stand down while you are typing, so `[` in your notes is just a bracket. The
exceptions are deliberate and named in the list — **Jump to…** works mid-sentence, because it is how
you leave where you are.

Undo is the interesting one: inside a text box, `Mod+Z` belongs to the box, not to your world.
Taking it would rewind an entire save every time somebody tried to take back a word.

> **Two commands, one shortcut** You are allowed to use a combination that is already taken, and
> Wobu tells you what happened rather than letting you find out by pressing it: the first command
> wins, the others do nothing, and both the editor and the shortcuts list name the winner and offer
> to put things back. Combinations your system already uses — copy, paste, quit, reload — are
> allowed too, with a warning that the system will act as well.

## Settings

| Section | What is in it |
| --- | --- |
| Providers and models | What this world uses, and the keys on this computer. See [Providers and keys](providers.md). |
| Legal | The terms and the privacy policy, read from the files that came with the app, and what you agreed to. |
| Introduction | Runs the opening tour again. It never re-asks you to agree to anything. |
| Agent access (MCP) | Letting an AI assistant read or change your world. See [Agent access](agent-access.md). |
| Storage | Where the search index is, how big it is, and **Rebuild search index**. |
| Editor | How long Wobu waits before saving. Raise it on a slow shared drive. |
| Appearance | Theme — *Match system*, *Light* or *Dark* — and how big everything is. |
| Keyboard | Every shortcut, changeable, with clashes named. |
| Static world wiki | Exports your world as a browsable website. Only appears with a world open. |
| Diagnostics | Where the log file is and how big, how much it records, a button to show it in your file manager, and the last of it readable in the app. |
| About | Which version of everything you are running. |
| Licences | Wobu's own licence, and everything it is built on. |

> **Rebuilding the search index is always safe** It holds no original of anything — it is made from
> your files. Delete it or rebuild it and nothing is lost; the next open just takes longer. If
> search misses something, or a file seems stuck, rebuild before you go looking anywhere else.

### The website export

**Export static wiki…** writes out a plain website — no clever machinery, nothing to install — with
a page per entity carrying its notes, description, facts, pictures, concepts and connections, an
index grouped by sort, and a diagram of how everything influences everything else. It only reads
your world, so it works on a read-only folder, and it refuses to write into a folder that already
exists or that sits inside your world.

## What is in the folder

A world is a self-contained folder, not one big database file. Put it on a NAS, a USB stick, or in
version control.

```
Ashfall.wobu/
├── project.json                  name, id, which services it uses
├── nodes/
│   ├── style-guide/              the Art Style page
│   ├── world-bible/              the World Canon page
│   ├── species/vashk.md          settings at the top, then notes, then description
│   ├── culture/ember-guild.md
│   ├── setting/cinder-bay.md
│   ├── character/kael-vantris.md
│   └── creature/  prop/  environment/  vehicle/
├── assets/
│   ├── originals/a3/a3f9…c1.png  filed by contents, in folders by the first two characters
│   ├── thumbs/a3/a3f9…c1.webp
│   ├── loras/7d/7d42…9e.safetensors
│   └── meshes/7b/7b21…04.glb
├── generations/2026-07/<ulid>.json
└── .wobu/
    ├── sessions/<session-id>.json  who else has this open right now
    └── tmp/                        where saves are staged
```

**The text files are the real thing.** Everything else can be rebuilt from them. You get version
history on your world, the ability to edit it in any other app, backups that just work, and no
dread about upgrading.

### Rules that keep the folder portable

| | |
| --- | --- |
| Nothing knows where it lives | The same shared drive has one address on your machine and a different one on somebody else's, so nothing inside ever writes an address down. |
| Pictures filed by contents | Two people adding the same reference end up with the same one file. Duplicates cost nothing and pictures never clash. |
| Cautious filenames | Lowercase, plain, nothing Windows or a shared drive would reject, and not nested deeply enough to hit a length limit. |
| No shortcuts or links | They do not survive shared drives, zip files, or most sync apps. |
| Records are written once | Never changed afterwards, so there is nothing to clash over. |
| No keys, ever | Those are in your computer's password store. The world file only records which service it prefers. |

### What is deliberately *not* in the folder

The search index, your shortcuts, your favourites, your theme, where your ComfyUI is, the assistant
password and the fact that you agreed to the terms all live on this computer instead — because each
one is a fact about you or your machine, not about the world. Put any of them in the folder and they
become everybody's.

The search index in particular lives with the app, filed by the world's id rather than its location,
so it survives the shared drive being remounted somewhere else. That sort of file is known to get
quietly corrupted on network drives, which is exactly why it is kept off them. Copy a world folder
to a computer that has never seen it and nothing is lost — the first open rebuilds.

## When something is wrong

| What you see | What to try |
| --- | --- |
| Search misses something you know is there | Settings → Storage → Rebuild search index. |
| A page shows as broken | Show it in the folder and open the file in a text editor — usually a hand edit or a sync app that broke the settings block at the top. Wobu will not write over it until it makes sense again. |
| Everything has gone read-only | Check the folder's permissions. Wobu decides this once, when the world opens. |
| A banner says the drive is offline | Wobu is trying again, waiting longer each time. The app stays readable from its own copy, and held saves go through when the drive comes back. |
| Enhance or Generate is greyed out | No key on this computer for the service chosen, or the folder is read-only. Settings → Providers and models. |
| Generate fails on a Gemini key that works | Image models need billing turned on; text is free. See [Providers and keys](providers.md). |
| 3D says your credentials are wrong for no reason | Check your computer's clock — signatures expire after a few minutes of drift. |
| A job failed and you missed the message | The notifications panel in the bottom bar keeps every failure, with what it cost. |
| Reference pictures seem to do nothing | Check the job you gave them, and check what the service accepts — the panel reports every picture it did not send. |
| Typing feels laggy on a NAS | Raise the save delay in Settings → Editor. |
| Something needs reporting | Settings → Diagnostics. Turn the detail up, make it happen again, then read the log in the app before you send it — keys are blanked out, but you should still see what you are handing over. |

## Glossary

**These are the words the app itself uses.** One thing, one word: if a term is in here, that is what
every label, tooltip, empty screen and error message says, and what this guide says too.

### Your world

| | |
| --- | --- |
| World | Everything in one project: the tree, its pictures, and everything made from it. |
| Entity | Any page in the world — a species, a character, a prop, and the two pinned ones all work the same way. |
| Kind | What sort of page something is. It decides the icon, the colour, where it sits in a prompt, which sections its description has, and what it can be joined to. |
| Singleton | Art Style and World Canon — one of each per world, pinned above the line. |
| Nesting | A page inside another of the same sort: a district in a city, a region in a region. |
| Link | A connection from one page to another, with a job and a strength. Links go between sorts; nesting does not. |
| Attributes | The short facts on a page — height, era, a colour — as opposed to the writing. |
| Stale | A description whose notes, or something above it, changed since it was last enhanced. |

### Turning a world into a prompt

| | |
| --- | --- |
| Influence stack | Everything feeding the picture you are about to make, in order. |
| Layer | One rung of that: style, world, ancestry, culture, place, subject, shot. |
| Fragment | One line of text, or one picture, put in by a layer — with a strength and somewhere to go. |
| Compiled prompt | The finished prompt, with every part of it traceable to the page that wrote it. |
| Enhance | Turning your rough notes into a description in named sections. Nothing is saved until you have read it. |
| Preset | What sort of picture you want: which parts to lean on, how it is framed, what shape, how many. |
| Influence snapshot | Exactly what went into a picture, saved with it, so the same thing can be made again later. |

### Pictures

| | |
| --- | --- |
| Reference | A picture attached to a page to steer what it looks like. |
| Role | What a reference is *for* — silhouette, palette, material, mood, pose, costume, or full reference. |
| Cover | The picture shown on a page's card and its row in the list. |
| Asset | Any file in the world's own store: a reference, a picture you made, a 3D shape, a trained look. |
| Unused | An asset nothing points at and nothing uses as a cover. The only kind Wobu offers to delete. |
| Concept | A picture you made, kept on the page you made it for. |
| Pinning | Turning a concept into a reference on its page, so it feeds the next picture. |
| Seed | The number that decides the random part of a picture. Lock it to get the same one again. |
| Turnaround | Eight views of one thing, on one locked seed — what a 3D shape is built from. |
| Take | One attempt at a single turnaround view. Every attempt is kept. |
| LoRA | A small trained file that teaches an image model what one particular thing looks like. |

### Getting things made, and what it costs

| | |
| --- | --- |
| Provider | A service that does the work — Anthropic, Gemini, a ComfyUI you run, Tencent. You bring your own key for each. |
| Model | The particular model at a service. One service usually offers several. |
| Job | One thing Wobu has asked a service for: a picture, an enhance, a 3D shape. |
| Queue | The jobs that have not finished yet. How many is in the bottom bar. |
| Receipt | What Wobu keeps of a finished picture: the prompt, the settings, everything that went into it and what it cost. Written once and never changed. |

### Files, folders and other people

| | |
| --- | --- |
| Project | One folder holding a whole world. Not a database file. |
| Search index | Wobu's own quick copy of the world, for searching and for reading fast. It holds no original of anything, so rebuilding it is always safe. |
| Read-only | A world folder Wobu cannot write to. Everything is readable; everything that saves is switched off. |
| Two versions | What you get when somebody else saved the same file first. Wobu keeps both and asks which you want; it never merges and never overwrites. |
| Ticket | What lets somebody else's Wobu join this world. Made from the world menu, accepted on the opening screen, and to be shared privately. |

## For the curious

This guide describes what Wobu does. Why it does it that way — and what forced each decision — is
written up in the design documents that live with the source code: [Vision](../01-vision.md), [Data
model](../02-data-model.md), [UI layout](../03-ui-layout.md), [Influence
engine](../04-influence-engine.md), [Architecture](../05-architecture.md), [File
shares](../07-file-shares.md), [Providers](../08-providers.md), [MCP](../16-mcp.md),
[Roadmap](../09-roadmap.md).
