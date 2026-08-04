import { site } from '../lib/site.mjs'

/*
 * Every claim below is taken from `README.md`, `docs/01-vision.md` and
 * `docs/09-roadmap.md`. Feature status is deliberately hedged where the
 * roadmap hedges it — Concept 3D is view/export-only today.
 *
 * The reader is a world builder, not an engineer: a GM writing a setting, a
 * small game team, a novelist with a map. Keep the words they would use.
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
    title: 'Say it once, in the right place',
    body: `Your art style belongs to the whole world. Build and anatomy belong to the species.
      Clothing belongs to the culture. Only what makes someone <em>them</em> belongs on the
      character. Do that and your hundredth character is quicker to make than your first — and fits
      in better.`,
  },
  {
    title: 'Scribbled notes in, proper description out',
    body: `Write the way you would in a notebook. Press <em>Enhance</em> and your notes come back
      as a clear description — shape, build, materials, colours, and a list of things to never
      draw. You read it and choose what to keep. Your own notes are never touched.`,
  },
  {
    title: 'Nothing is hidden from you',
    body: `The prompt is always on screen, and every phrase in it is colour-coded to show which
      page it came from. Turn any part of your world down, or off, for one picture without
      changing anything you have written.`,
  },
  {
    title: 'Pictures count as description too',
    body: `Every reference picture you add gets a job — a shape to keep, a colour palette, a
      fabric, a pose, or just something for your own eyes. Wobu uses each one the way you meant
      it, instead of throwing them all in together.`,
  },
  {
    title: 'Your own graphics card, or a paid service',
    body: `Make pictures on your own machine through ComfyUI, or through Google Gemini. Everything
      you make is kept with the exact settings that made it, so you can compare, try again, or
      redo it months later.`,
  },
  {
    title: 'Work with other people, no server',
    body: `Put the world on a shared drive and a few of you can work in it at once, without
      standing on each other's toes. Or send someone a ticket and your two copies keep each other
      up to date, machine to machine. Nothing passes through us.`,
  },
]

const STEPS = [
  {
    title: 'Write your world down once',
    body: `Make a <code>.wobu</code> project and fill in two pages: how your world looks, and what
      is true in it. Then add your peoples, their cultures, their places and your characters, a
      page each. They are ordinary text files you could still read if Wobu vanished tomorrow.`,
  },
  {
    title: 'Tidy up the messy bits',
    body: `Half-finished notes become a proper description, using your own Claude or Gemini
      account. You get a side-by-side look at what changed and decide what to keep — nothing is
      saved until you say so.`,
  },
  {
    title: 'Make a picture',
    body: `Press Generate on anyone in your world and Wobu writes the prompt for you, gathering
      every layer above them and the reference pictures that belong. Like what comes out? Pin it,
      and everything below that point starts to look more like it.`,
  },
]

const NOT = [
  ['Not a finishing tool.', 'You get concept art and rough 3D shapes to hand to an artist.'],
  ['Not a wiki or a writing app.', 'Notes are here to shape pictures.'],
  ['Not a node graph.', 'ComfyUI already does that beautifully, and Wobu can drive it.'],
  ['Not a service.', 'No account to make, nothing to sign in to, nothing to subscribe to.'],
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
          The prompt Wobu wrote for one character. Each part is labelled and tinted with the page it
          came from, and you can turn any of them down — or off — for a single picture.
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
          <p class="eyebrow">Runs on your computer · Uses your own AI account · Free and open source</p>
          <h1>Write your world down once.<br />Every picture remembers it.</h1>
          <p class="lede">
            Wobu is a desktop app for people building worlds — a campaign setting, a game, a story.
            Describe your art style, your peoples, their cultures, their places and your characters
            once. From then on, every picture you make already knows all of it.
          </p>
          <p class="cta-row">
            <a class="btn btn-primary" href="download.html">Download the beta</a>
            <a class="btn" href="guide/index.html">Read the guide</a>
          </p>
          <p class="hero-note">
            Still in beta, and the downloads are not signed yet — so your computer will warn you
            about them. Only ever get Wobu from
            <a href="${site.releases}" rel="noopener">GitHub Releases</a>.
          </p>
        </div>
      </section>

      <section class="band" aria-labelledby="problem-heading">
        <div class="wrap">
          <h2 id="problem-heading">A world is a family tree, not a pile of prompts</h2>
          <p class="measure">
            A character comes from a people. That people lives in a world. The whole thing is drawn
            in one look. Right now all of that sits in your head, and you retype a rough version of
            it into every prompt — a little differently each time. That is why two people of the same
            species end up looking unrelated, why the light keeps drifting, and why by the fortieth
            picture your world has stopped looking like one place.
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
              <h2 id="local-heading">It all stays on your computer</h2>
              <p>
                A world is just a folder, ending in <code>.wobu</code>. The notes are plain text and
                the pictures are ordinary image files, so you can open them in anything, back them
                up, drop them on a USB stick or keep them in version control. Uninstall Wobu and your
                world is still sitting there, still readable.
              </p>
              <p>
                We run no servers. There is no account, no tracking, no crash reports and no update
                check — the app has nowhere to phone home to. Your work leaves your computer only
                when you press Enhance or Generate, and then it goes straight to the service you
                chose, on your own account. Your keys are kept in your computer's own password
                store, never in the world folder.
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
            Wobu is in beta and worked on most days. Building your world, sharing it, syncing between
            two machines, reference pictures, Enhance and the whole picture-making loop all work
            today — as do Forge, variant grids, teaching a model one character's look, borrowing a
            style from another world, scenes with several characters in them, and exporting your
            world as a browsable website. Concept 3D is half there: Wobu can show and export meshes,
            but it cannot yet start one from a turnaround for you.
          </p>
          <p>
            <a class="text-link" href="${site.roadmap}" rel="noopener"
              >See exactly what is finished →</a
            >
          </p>
        </div>
      </section>

      <section class="band cta-band" aria-labelledby="download-heading">
        <div class="wrap">
          <h2 id="download-heading">Try it</h2>
          <p class="measure">
            There are installers for Windows, macOS and Linux. If you would rather build it
            yourself, the whole thing is on GitHub.
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
      'Wobu is a desktop app for building a world and making concept art that stays consistent. ' +
      'Describe your world once; every picture you make already knows it.',
    main,
  }
}
