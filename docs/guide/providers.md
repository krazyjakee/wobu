# Providers and keys

Wobu ships with no inference of its own and no Wobu-operated proxy. You paste credentials you
obtained yourself, and Wobu talks to those providers directly on your behalf. Nothing is
pre-configured, and there is no account to make.

## Three capabilities, not one setting

Providers are chosen per *capability*, not globally. Enhancing with one vendor, generating images on
a local GPU, and producing meshes through a third service is a normal configuration, not an exotic
one.

| Capability | Used by | Providers |
| --- | --- | --- |
| Text | Enhance | Anthropic, Gemini |
| Image | Generate | ComfyUI, Gemini |
| Mesh | Concept 3D | Tencent Hunyuan3D, Local 2.1 (ComfyUI) |

Text also has a free-text **Model** field, because model names move faster than releases do. Leave
it blank for the provider's default.

## Two bands, because they belong to different people

Settings → Providers and models is split in half, and the split is the point.

**What this project uses** is written into `project.json` and travels with the folder. Opening a
shared world shows you the choices whoever built it made — provider, model, mesh region, spend
ceiling. It is tagged `shared`.

**Keys on this computer** never travel. They are tagged `local`, and they are listed once per vendor
rather than once per capability, because a key is not per capability: the same Gemini key writes
text and makes pictures. The ComfyUI address lives in this band too, for the same reason — it
describes this computer, not the shared world.

## How keys are stored

Keys go into your **operating system keychain** — Secret Service on Linux, Keychain on macOS,
Credential Manager on Windows. They are held in Wobu's native process only: never sent to the
webview, never written to a log, and redacted from error messages before those reach the interface.
A key present in the environment is also honoured, and Settings says which of the two a credential
came from.

> **Keys are never written into the project folder** Project folders are designed to live on shared
> drives. A key in `project.json` would be a key leaked to everyone with share access — and to git
> history, and to whoever the folder eventually gets zipped to. So the project file records only the
> *selection*: provider id, model id, region and default parameters.

The consequence is that **keys are per-installation, not per-project**. Opening a shared project
uses *your* keys. A collaborator without one sees `Gemini selected — no key on this machine` with an
**Add key** button, rather than a failure once a job is already running.

Where a provider supports it, **Check this key** asks for one description and stops it after a
couple of dozen tokens — a fraction of a penny, and nothing at all if the key is refused.

### Missing keys degrade gracefully

Opening a project whose selected provider has no key on this machine drops that one capability into
a disabled state with a direct affordance to fix it. You can still read, write and organise the
whole world — only Enhance or Generate goes quiet. On Linux, a locked login keyring is reported as
exactly that, rather than as a failed save.

## Everything network happens in Rust

Every provider request is issued from Wobu's native process, never from the web view. Three reasons,
in order of importance:

1. The key never enters renderer memory.
2. Streaming, cancellation and retry are already the job queue's problem.
3. It sidesteps browser CORS behaviour that breaks at least one vendor's API from inside a desktop
   web view entirely.

## Anthropic

Keys come from the Claude Console. Anthropic is the text default: a project that has chosen nothing
uses it, and Settings says so rather than leaving the field ambiguous.

## Google Gemini

Keys come from AI Studio. Gemini can serve both the text and image capabilities.

### Text — for Enhance

A fast Flash-tier model by default. Structured descriptions are requested with a response schema so
what comes back is schema-valid JSON rather than prose that has to be parsed.

> **The thing most likely to confuse you** **Gemini image generation has no free tier.** Text is
> free; every image model is billing-only. If you paste a working key, successfully enhance a
> species, and then hit an error on Generate, nothing is broken — you need billing enabled on your
> Google account. Wobu detects this specific case and says so plainly rather than surfacing a raw
> 403.

### Image — for Generate

Several tiers, from a cheap lite model up to a high-quality one with the largest reference budget.
Images return inline rather than as URLs, and all output carries SynthID watermarking.

Two capability facts worth knowing before you plan around them:

- **Reference budgets differ per model**, per bucket — objects, characters and style references —
  and Wobu's image budget respects them. See [References](references.md).
- **Gemini image models take no negative prompt.** Every *Never* fragment is withheld and reported
  as withheld, rather than being quietly pasted into the positive prompt where it would summon the
  thing you excluded.

## ComfyUI (local)

Point Wobu at a running ComfyUI instance and generation happens on your own GPU. No key, no cost
estimate, no per-image billing, and nothing leaves the machine. The default address is
`http://127.0.0.1:8188`; **Save and check** verifies it. It is stored in Wobu's application data on
this computer, never in the project folder.

A non-loopback server receives the prompts and reference images those jobs need, so enter only a
server you trust. Credentials in the URL are rejected; an authenticating proxy has to be configured
outside Wobu.

Wobu drives ComfyUI rather than replacing it — the node graph stays where it is, and Wobu's surface
remains a document editor.

> **Two current limits of the local route** The shipped workflows have no image input, so
> **reference images are not sent to ComfyUI at all** — they are reported as not sent rather than
> silently ignored. And Flux-family checkpoints have no negative conditioning, so *Never* fragments
> are withheld for those models too. Local ComfyUI does stream live previews, which the hosted image
> providers do not.

## Tencent Hunyuan3D (mesh)

The hosted 3D backend, and the one that is genuinely more work to set up. Settings walks it in three
linked steps — activate the service, open CAM users, create a sub-account key — and it is worth
knowing why before you start:

| | |
| --- | --- |
| No bearer API key | Authentication is a `SecretId` and `SecretKey` pair with request signing. Wobu handles the signing; you paste both halves. |
| Use a scoped sub-account key | The account `SecretKey` is an account-wide master credential, not a scoped token — materially more dangerous to hold than a typical API key. Create a sub-account key scoped to the 3D policy rather than pasting your root credentials. |
| Region is required | Only three regions work: Singapore, Silicon Valley and Frankfurt. Concept 3D stays off until the project records one; Wobu will not guess where to send your images, and every poll stays in the region its job was submitted to. |
| Activate the service first | A fresh account fails until the 3D service is explicitly activated in the provider's console. Wobu treats this as an onboarding step with a link, not a runtime error. |
| Check your system clock | Signatures expire after a few minutes of clock skew. Desktop clocks drift, so Wobu maps that specific failure to a "check your system clock" message rather than an opaque auth error. |

## Capability negotiation

Each backend declares what it can do, and the generation surfaces consume those declarations before
queueing rather than discovering the answer from an error:

- Aspect lists come from the selected backend; unsupported saved values are repaired before
  queueing, and the negotiated dimensions are shown.
- A backend that accepts no structure references shows yours as downgraded to mood-board-only.
- Per-bucket reference caps drive the image budget, so the inspector can say `3/3 style refs` and
  name what got dropped.
- A backend with no negative-prompt support has its negative withheld and says so.
- Backends that require billing are marked as such before you spend anything.

## Spend control

- Estimated cost on the Generate button for paid providers; nothing shown for local ones.
- A **shared spend ceiling** per project, with a hard stop, recorded in the project so it applies to
  everyone who opens it.
- Every generation record stores its provider, model and parameters, so real spend is
  reconstructable from the project folder — you are never dependent on a vendor dashboard to know
  what a world cost to make.
- Mesh reconstruction is the exception, and is gated on an explicit consent tick instead: its
  provider does not report what it charged. See [Concept 3D](concept-3d.md).
