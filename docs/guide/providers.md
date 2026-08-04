# Providers and keys

Wobu does no AI work of its own, and sells you nothing. You paste in a key you got yourself, and
Wobu talks to that company directly on your behalf. Nothing is set up in advance, and there is no
account to make here.

## Three jobs, not one setting

You pick a service for each *job*, not one for everything. Enhancing with one company, making
pictures on your own graphics card, and making 3D shapes through a third is a perfectly ordinary
setup.

| Job | Used by | Services |
| --- | --- | --- |
| Text | Enhance | Anthropic, Gemini |
| Images | Generate | ComfyUI, Gemini |
| 3D | Concept 3D | Tencent Hunyuan3D, Local 2.1 (ComfyUI) |

Text also has a **Model** box you can type into, because model names change faster than apps do.
Leave it blank for the company's default.

## Two halves, because they belong to different people

Settings → Providers and models is split down the middle, and the split is the whole point.

**What this project uses** is saved in the world folder and travels with it. Open a world somebody
else built and you can see the choices they made — which service, which model, which region, the
spending limit. It is tagged `shared`.

**Keys on this computer** never travel. They are tagged `local`, and they are listed once per
company rather than once per job, because a key is not tied to a job — the same Gemini key writes
text and makes pictures. Where your ComfyUI is running lives in this half too, for the same reason:
it describes this computer, not the world.

## How keys are stored

Keys go into your computer's own password store — Secret Service on Linux, Keychain on macOS,
Credential Manager on Windows. Only the part of Wobu that talks to the internet ever sees them: they
never reach the part that draws the screen, are never written to a log file, and are blanked out of
error messages before you can read them. A key already set up in your environment is used too, and
Settings tells you which of the two a key came from.

> **Keys never go in the world folder** World folders are meant to live on shared drives. A key in
> there would be a key handed to everybody with access to that drive — and to your version history,
> and to whoever the folder eventually gets emailed to. So the world only records the *choice*:
> which service, which model, which region, and the default settings.

That means **keys belong to your installation, not to a world**. Opening somebody else's world uses
*your* keys. A collaborator without one sees `Gemini selected — no key on this machine` and an **Add
key** button, rather than a failure halfway through a job.

Where a company allows it, **Check this key** asks for one short description and stops after a few
dozen words — a fraction of a penny, and nothing at all if the key is refused.

### Missing keys are not a disaster

Open a world whose chosen service you have no key for and just that one job goes quiet, with a
button right there to fix it. You can still read, write and organise the whole world — only Enhance
or Generate stops. On Linux, a locked login keyring is reported as exactly that, rather than as a
failed save.

## Anthropic

Keys come from the Claude Console. Anthropic is the default for text: a world that has not chosen
anything uses it, and Settings says so rather than leaving the box looking empty.

## Google Gemini

Keys come from AI Studio. Gemini can do both the text and the pictures.

### Text — for Enhance

A fast, cheap model by default. Descriptions are asked for in a fixed shape, so what comes back
fits neatly into sections rather than being prose somebody has to unpick.

> **The thing most likely to catch you out** **Gemini image generation has no free tier.** Text is
> free; every image model needs billing. So if you paste in a working key, enhance a species
> perfectly happily, and then hit an error on Generate — nothing is broken. You need billing turned
> on in your Google account. Wobu spots this exact case and says so plainly instead of showing you a
> raw error.

### Images — for Generate

Several tiers, from a cheap small model up to a good one that takes the most reference pictures.
Pictures come back directly rather than as links, and everything carries an invisible SynthID
watermark.

Two things worth knowing before you plan around them:

- **How many reference pictures a model takes varies**, and it varies by type — objects, characters
  and style. Wobu respects those limits. See [References](references.md).
- **Gemini image models take no "never draw this" list.** Every *Never* line is held back and
  reported as held back, rather than being quietly pasted into the prompt, where it would summon
  the very thing you banned.

## ComfyUI (on your own machine)

Point Wobu at a running ComfyUI and pictures are made on your own graphics card. No key, no
estimate, no bill, and nothing leaves the machine. The usual address is `http://127.0.0.1:8188`;
**Save and check** confirms it is really there. It is remembered on this computer, never in the
world folder.

If you point it at a machine other than your own, that machine receives your prompts and reference
pictures — so only ever use a server you trust. Passwords in the address are refused; anything
needing a login has to be set up outside Wobu.

Wobu drives ComfyUI rather than replacing it. Your node graph stays exactly where it is, and Wobu
stays a document editor.

> **Two limits worth knowing** The setups that ship with Wobu take no picture input, so **reference
> pictures are not sent to ComfyUI at all** — they are reported as not sent rather than quietly
> ignored. And Flux-family models have no way to take a "never draw this" list, so those lines are
> held back for them too. On the plus side, ComfyUI shows you a live preview while it works, which
> the paid services do not.

## Tencent Hunyuan3D (3D shapes)

The paid 3D service, and the one that genuinely takes some setting up. Settings walks you through it
in three linked steps — switch on the service, open the users page, make a limited key — and it is
worth knowing why before you start:

| | |
| --- | --- |
| There is no simple API key | It uses an id and a secret together, with each request signed. Wobu does the signing; you paste in both halves. |
| Make a limited key, not a master one | The account-level secret unlocks your whole account, not just this one service — a lot more dangerous to have lying around than a normal key. Make a sub-account key limited to 3D instead of pasting in your main credentials. |
| A region is required | Only three work: Singapore, Silicon Valley and Frankfurt. Concept 3D stays off until the world records one, because Wobu will not guess where to send your pictures — and it always asks about a job in the same region it sent it to. |
| Switch the service on first | A brand new account fails until you explicitly turn the 3D service on in their console. Wobu treats that as a setup step with a link, not as an error. |
| Check your computer's clock | Signatures expire after a few minutes of drift. Desktop clocks do drift, so Wobu turns that particular failure into "check your system clock" rather than a baffling authentication error. |

## Wobu asks first, rather than finding out the hard way

Each service says up front what it can do, and Wobu uses that before sending anything rather than
discovering it from an error:

- The list of shapes comes from the service you chose; a saved shape it will not accept is fixed
  first, and you are shown the size it settled on.
- A service that takes no shape references shows yours as downgraded to mood-board-only.
- Limits on reference pictures are respected, so the panel can say `3/3 style refs` and name what
  got left out.
- A service with no "never draw this" list has yours held back, and says so.
- Services that require billing are marked as such before you spend anything.

## Keeping a lid on spending

- An estimate on the Generate button for paid services; nothing at all for your own machine.
- A **spending limit** per world, with a hard stop, saved in the folder so it applies to everyone
  who opens it.
- Every picture is saved with the service, model and settings that made it, so what a world really
  cost can be worked out from the folder — you never have to log into anybody's billing page to find
  out.
- 3D is the exception, and sits behind a tick box instead: that service does not report what it
  charged. See [Concept 3D](concept-3d.md).
