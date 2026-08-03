import { site } from '../lib/site.mjs'

/*
 * No version numbers and no direct asset URLs: `/releases/latest` always
 * resolves to the current published release, and the bundle filenames carry a
 * version that a hand-written link would be wrong about within a day.
 * Install guidance is condensed from `docs/12-releasing.md`.
 */

const PLATFORMS = [
  {
    name: 'Linux',
    artefacts: '<code>.AppImage</code>, <code>.deb</code>, <code>.rpm</code>',
    note: `Prefer the package for your distribution. For the AppImage, make it executable with
      <code>chmod +x Wobu_*.AppImage</code> and run it. Linux packages are not repository-signed,
      so your package manager cannot establish publisher identity.`,
  },
  {
    name: 'macOS',
    artefacts: '<code>.dmg</code>',
    note: `Drag Wobu into Applications. On first launch, control-click Wobu and choose
      <strong>Open</strong>, then confirm <strong>Open</strong> — an unsigned app has no
      double-click path past Gatekeeper.`,
  },
  {
    name: 'Windows',
    artefacts: '<code>.msi</code>, <code>.exe</code>',
    note: `Run the installer. If SmartScreen appears, check that the publisher is shown as unknown,
      select <strong>More info</strong>, then <strong>Run anyway</strong>. An unexpected named
      publisher is a reason to stop, not an improvement.`,
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
            Every bundle is built by GitHub Actions from a tagged commit and published on the
            repository's releases page. There is no other download host, and no installer here
            phones home.
          </p>
        </div>
      </section>

      <section class="band" aria-labelledby="beta-heading">
        <div class="wrap">
          <div class="callout" role="note">
            <h2 id="beta-heading">Beta bundles are unsigned</h2>
            <p>
              Wobu does not yet hold Apple Developer ID or Windows code-signing credentials, so the
              release workflow deliberately holds no signing secrets. macOS Gatekeeper and Windows
              SmartScreen will warn. Confirm that a download came from
              <a href="${site.releases}" rel="noopener">this repository's releases page</a> before
              bypassing any warning, and never install a Wobu bundle copied to another host.
            </p>
            <p>
              Updates are manual: there is no updater endpoint and no update check. Install a newer
              release over the old one — your projects are ordinary folders and live outside the
              application.
            </p>
            <p>
              <a class="text-link" href="${site.releaseGuide}" rel="noopener"
                >Full install guidance →</a
              >
            </p>
          </div>
        </div>
      </section>

      <section class="band" aria-labelledby="platforms-heading">
        <div class="wrap">
          <h2 id="platforms-heading">Choose a platform</h2>
          <ul class="cards">
          ${platforms}
          </ul>
          <p class="measure">
            Pick the asset matching your platform from the release's <strong>Assets</strong> list.
            Compare the release tag with the version shown in Wobu's About panel after installing.
          </p>
        </div>
      </section>

      <section class="band band-alt" aria-labelledby="source-heading">
        <div class="wrap">
          <h2 id="source-heading">Or run it from source</h2>
          <p class="measure">
            You need Node.js 22 and npm, the Rust toolchain (the pinned version installs itself via
            <code>rustup</code>), and the
            <a href="https://v2.tauri.app/start/prerequisites/" rel="noopener"
              >Tauri 2 system prerequisites</a
            >
            for your platform.
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
          <h2 id="after-heading">After it opens</h2>
          <p class="measure">
            Create a project from the Launcher, start with its <strong>Style Guide</strong> and
            <strong>World Canon</strong>, add broad world layers before individual characters, then
            add your provider keys in Settings. Keys are stored in the operating-system keychain,
            never inside a project folder.
          </p>
          <p class="cta-row">
            <a class="btn btn-primary" href="guide/index.html">Read the user guide</a>
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
      'Download Wobu for Linux, macOS or Windows from GitHub Releases, or build and run it from ' +
      'source. Beta bundles are unsigned and update manually.',
    main,
  }
}
