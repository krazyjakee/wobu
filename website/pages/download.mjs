import { site } from '../lib/site.mjs'

/*
 * No version numbers and no direct asset URLs: `/releases/latest` always
 * resolves to the current published release, and the bundle filenames carry a
 * version that a hand-written link would be wrong about within a day.
 * Install guidance is condensed from `docs/12-releasing.md`.
 */

const PLATFORMS = [
  {
    name: 'Windows',
    artefacts: '<code>.msi</code>, <code>.exe</code>',
    note: `Either one works — take the <code>.msi</code> if you are not sure. Run it. Windows will
      say the publisher is unknown, because we have not paid for a signature yet: choose
      <strong>More info</strong>, then <strong>Run anyway</strong>. If it ever names a publisher
      you have not heard of, stop — that is not us.`,
  },
  {
    name: 'macOS',
    artefacts: '<code>.dmg</code>',
    note: `Open it and drag Wobu into Applications. The first time you open it, right-click the app
      and choose <strong>Open</strong>, then confirm. Double-clicking will not work until you have
      done that once — macOS blocks apps that have not been signed.`,
  },
  {
    name: 'Linux',
    artefacts: '<code>.AppImage</code>, <code>.deb</code>, <code>.rpm</code>',
    note: `Take the <code>.deb</code> on Debian or Ubuntu, the <code>.rpm</code> on Fedora, and the
      <code>.AppImage</code> anywhere else — make it runnable with
      <code>chmod +x Wobu_*.AppImage</code> and open it.`,
  },
]

export function downloadPage() {
  const platforms = PLATFORMS.map(
    (platform) => `<li class="card">
            <h3>${platform.name}</h3>
            <p class="artefacts">${platform.artefacts}</p>
            <p>${platform.note}</p>
            <p>
              <a class="btn btn-primary btn-small" href="${site.latestRelease}" rel="noopener"
                >Latest release</a
              >
            </p>
          </li>`,
  ).join('\n          ')

  const main = `      <section class="page-head">
        <div class="wrap">
          <p class="eyebrow">Download</p>
          <h1>Get Wobu</h1>
          <p class="lede">
            Wobu is free. Every download is built straight from the public source code and published
            on GitHub — there is nowhere else to get it, and nothing in the installer talks back to
            us.
          </p>
        </div>
      </section>

      <section class="band" aria-labelledby="beta-heading">
        <div class="wrap">
          <div class="callout" role="note">
            <h2 id="beta-heading">Your computer will warn you. Here is why.</h2>
            <p>
              Signing an app costs money every year, and Wobu has not paid it yet. So Windows and
              macOS cannot tell who made the file, and they say so — loudly. That warning is doing
              its job. Before you click past it, check the download actually came from
              <a href="${site.releases}" rel="noopener">the releases page</a>, and never install a
              copy of Wobu somebody sent you from anywhere else.
            </p>
            <p>
              There is no automatic updating, and Wobu never checks for it. When a new version comes
              out, download it and install it over the old one. Your worlds are ordinary folders
              somewhere else on your disk, so nothing of yours is touched.
            </p>
            <p>
              <a class="text-link" href="${site.releaseGuide}" rel="noopener"
                >More detail on installing →</a
              >
            </p>
          </div>
        </div>
      </section>

      <section class="band" aria-labelledby="platforms-heading">
        <div class="wrap">
          <h2 id="platforms-heading">Pick your computer</h2>
          <ul class="cards">
          ${platforms}
          </ul>
          <p class="measure">
            The button takes you to the newest release. Scroll to <strong>Assets</strong> and pick
            the file for your machine.
          </p>
        </div>
      </section>

      <section class="band band-alt" aria-labelledby="source-heading">
        <div class="wrap">
          <h2 id="source-heading">Or build it yourself</h2>
          <p class="measure">
            For the technically inclined. You will need Node.js 22, the Rust toolchain (it installs
            its own version), and the
            <a href="https://v2.tauri.app/start/prerequisites/" rel="noopener"
              >Tauri 2 prerequisites</a
            >
            for your system.
          </p>
          <pre class="code"><code>git clone ${site.repo}.git
cd wobu
npm ci
cd src-tauri &amp;&amp; rustup toolchain install --no-self-update &amp;&amp; cd ..
npm run tauri dev</code></pre>
        </div>
      </section>

      <section class="band" aria-labelledby="after-heading">
        <div class="wrap">
          <h2 id="after-heading">Once it opens</h2>
          <p class="measure">
            Make a new world, and start with the two pages already waiting in it:
            <strong>Art Style</strong> for how everything should look, and
            <strong>World Canon</strong> for what is true in it. Add your peoples and places before
            your favourite character — that order pays off fast. Then drop in your AI account keys
            under Settings. They go into your computer's password store, never into the world folder.
          </p>
          <p class="cta-row">
            <a class="btn btn-primary" href="guide/index.html">Read the guide</a>
            <a class="btn" href="legal.html">Licence and legal</a>
          </p>
        </div>
      </section>`

  return {
    path: 'download.html',
    nav: 'download',
    depth: 0,
    title: 'Download',
    description:
      'Download Wobu free for Windows, macOS or Linux, or build it yourself. The beta is not ' +
      'signed yet, so your computer will warn you, and updating is a manual download.',
    main,
  }
}
