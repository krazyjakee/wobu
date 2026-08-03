import { site } from '../lib/site.mjs'

/*
 * Every claim below is taken from `README.md`, `docs/01-vision.md` and
 * `docs/09-roadmap.md`. Feature status is deliberately hedged where the
 * roadmap hedges it — Concept 3D is view/export-only today.
 */

const LAYERS = [
  { key: 'style', label: 'Style', text: 'gritty painterly concept art, heavy rim light' },
  { key: 'world', label: 'World', text: 'ash-choked post-eruption coast, late iron age' },
  { key: 'species', label: 'Species', text: 'tall narrow-shouldered digitigrade humanoid' },
  {
    key: 'culture',
    label: 'Culture',
    text: 'ember guild kiln-glaze plate, signet at the collarbone',
  },
  { key: 'place', label: 'Place', text: 'cinder bay harbour light, salt-bleached timber' },
  { key: 'subject', label: 'Subject', text: 'Kael Vantris, scarred, ex-guild, full body' },
]

const FEATURES = [
  {
    title: 'A hierarchy, not a prompt box',
    body: `Author style at the project level, anatomy at the species level, costume at the culture
      level, personality at the character level. The hundredth character is cheaper and more
      consistent than the first.`,
  },
  {
    title: 'Notes in, canon out',
    body: `Write rough, messy notes. <em>Enhance</em> turns them into a structured, editable
      description — silhouette, anatomy, materials, palette, signature details, never — using
      Anthropic or Gemini with your own key. Your original notes are never overwritten.`,
  },
  {
    title: 'Prompts you can audit',
    body: `The compiled prompt is always on screen and every fragment is attributed to the layer
      it came from. Mute or reweight any layer for a single generation without editing the world.`,
  },
  {
    title: 'Reference images with a role',
    body: `An image carries a role — silhouette, structure, palette, material, mood or pose — so
      the compiler knows whether to route it to a style adapter, a structure adapter, or the
      reference grid, and how much weight to give it.`,
  },
  {
    title: 'Local ComfyUI or hosted models',
    body: `Generate through a local ComfyUI installation or through Gemini, with per-entity
      generation history, replayable snapshots, variant grids, seed locking and pin-to-reference
      promotion.`,
  },
  {
    title: 'Share a folder, or a ticket',
    body: `A project can live on a file share with presence, conflict-safe atomic writes and
      conflict siblings; or you can sync directly with a peer over a ticket. No server in the
      middle either way.`,
  },
]

const STEPS = [
  {
    title: 'Author the world once',
    body: `Create a <code>.wobu</code> project and fill in its Style Guide and World Canon, then
      add species, cultures, places and characters as a tree. Notes are ordinary Markdown files
      you could read without Wobu installed.`,
  },
  {
    title: 'Enhance the messy parts',
    body: `Turn half-sentence notes into structured canonical descriptions with your own
      Anthropic or Gemini key. The result is editable and tracked; re-enhancing shows a diff you
      accept or reject.`,
  },
  {
    title: 'Generate with the whole chain',
    body: `Hit Generate on any node and the prompt is compiled from every layer above it, with
      per-role reference images budgeted alongside the text. Pin a result to promote it to a
      reference that then influences everything downstream.`,
  },
]

const NOT = [
  ['Not a finishing pipeline.', 'Output is concept art and blockout meshes for a modeller.'],
  ['Not a wiki or a novel-writing app.', 'Notes exist to drive images.'],
  ['Not a node-graph tool.', 'ComfyUI already exists, and Wobu can drive it.'],
  ['Not a service.', 'There is no account to create and no server to sign in to.'],
]

function prompt() {
  const fragments = LAYERS.map(
    (layer) => `<span class="frag frag-${layer.key}">
            <span class="frag-label">${layer.label}</span>
            <span class="frag-text">${layer.text}</span>
          </span>`,
  ).join('\n          ')

  return `<figure class="prompt-figure">
        <div class="prompt">
          ${fragments}
        </div>
        <figcaption>
          A compiled prompt for one character. Each fragment is labelled and tinted with the layer
          that contributed it, and every layer can be muted or reweighted for a single generation.
        </figcaption>
      </figure>`
}

export function homePage() {
  const features = FEATURES.map(
    (feature) => `<li class="card">
            <h3>${feature.title}</h3>
            <p>${feature.body}</p>
          </li>`,
  ).join('\n          ')

  const steps = STEPS.map(
    (step, index) => `<li class="step">
            <p class="step-number" aria-hidden="true">${index + 1}</p>
            <h3>${step.title}</h3>
            <p>${step.body}</p>
          </li>`,
  ).join('\n          ')

  const not = NOT.map(([term, detail]) => `<li><strong>${term}</strong> ${detail}</li>`).join(
    '\n            ',
  )

  const main = `      <section class="hero">
        <div class="wrap">
          <p class="eyebrow">Local-first · Bring your own key · MIT licensed</p>
          <h1>Author your world once.<br />Every image inherits it.</h1>
          <p class="lede">
            Wobu is a desktop app for building coherent fictional worlds and producing consistent
            concept art. Define lore, visual style, species, cultures, places, characters and props
            once; Wobu resolves that hierarchy into an attributed prompt whenever you generate an
            image.
          </p>
          <p class="cta-row">
            <a class="btn btn-primary" href="download.html">Download the beta</a>
            <a class="btn" href="guide/index.html">Read the guide</a>
          </p>
          <p class="hero-note">
            Beta software: release bundles are currently unsigned. Download them only from
            <a href="${site.releases}" rel="noopener">GitHub Releases</a>.
          </p>
        </div>
      </section>

      <section class="band" aria-labelledby="problem-heading">
        <div class="wrap">
          <h2 id="problem-heading">World building is a tree, not a list of prompts</h2>
          <p class="measure">
            A character belongs to a species, which belongs to a world, which is rendered in a house
            art style. Today that context lives in the artist's head and gets retyped — badly,
            inconsistently, differently every time — into each prompt. That is exactly where visual
            consistency dies: two characters of the same species end up looking unrelated, the
            lighting drifts, the palette drifts, and by image forty the world has no identity.
          </p>
          ${prompt()}
        </div>
      </section>

      <section class="band" aria-labelledby="how-heading">
        <div class="wrap">
          <h2 id="how-heading">How it works</h2>
          <ol class="steps">
          ${steps}
          </ol>
        </div>
      </section>

      <section class="band band-alt" aria-labelledby="features-heading">
        <div class="wrap">
          <h2 id="features-heading">What Wobu does</h2>
          <ul class="cards">
          ${features}
          </ul>
        </div>
      </section>

      <section class="band" aria-labelledby="local-heading">
        <div class="wrap">
          <div class="split">
            <div>
              <h2 id="local-heading">Local-first, and private by construction</h2>
              <p>
                A project is an ordinary, self-contained <code>.wobu</code> directory. Notes stay
                readable Markdown, assets stay files, and the disposable search index lives outside
                the project. It survives Wobu being uninstalled.
              </p>
              <p>
                Wobu operates no servers. There is no account, no inference proxy, no telemetry, no
                crash reporting and no update check — the application never contacts us, because
                there is nowhere for it to contact. Content leaves your machine only when you invoke
                a feature that uses a provider you configured, and then it goes directly to that
                provider under your own credentials, which are held in the operating-system
                keychain.
              </p>
              <p>
                <a class="text-link" href="legal/privacy-policy.html"
                  >Read the privacy policy →</a
                >
              </p>
            </div>
            <div class="panel">
              <h3>What Wobu is not</h3>
              <ul class="plain">
            ${not}
              </ul>
            </div>
          </div>
        </div>
      </section>

      <section class="band band-alt" aria-labelledby="status-heading">
        <div class="wrap">
          <h2 id="status-heading">Where it is up to</h2>
          <p class="measure">
            Wobu is in beta and under active development. The world tree, shareable projects,
            peer-to-peer sync, references, Enhance, the influence engine and the full generation
            loop are implemented; Forge, variant grids, per-entity LoRA training, cross-project
            style transfer, multi-entity scene composition and static wiki export ship too. Concept
            3D is partial — the app can view and export existing meshes, but starting a turnaround
            reconstruction from the UI is still planned.
          </p>
          <p>
            <a class="text-link" href="${site.roadmap}" rel="noopener"
              >Read the roadmap for exact feature status →</a
            >
          </p>
        </div>
      </section>

      <section class="band cta-band" aria-labelledby="download-heading">
        <div class="wrap">
          <h2 id="download-heading">Try it</h2>
          <p class="measure">
            Bundles are built for Linux, macOS and Windows from a tagged release, or you can run the
            app from source with Node 22 and the Rust toolchain.
          </p>
          <p class="cta-row">
            <a class="btn btn-primary" href="download.html">Download Wobu</a>
            <a class="btn" href="${site.repo}" rel="noopener">View the source</a>
          </p>
        </div>
      </section>`

  return {
    path: 'index.html',
    nav: 'home',
    depth: 0,
    title: site.name,
    description:
      'Wobu is a local-first desktop app for building coherent fictional worlds and producing ' +
      'consistent concept art. Author the hierarchy once; every generated image inherits it.',
    main,
  }
}
