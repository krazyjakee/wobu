import { useState } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'
import appLicence from '../../LICENSE?raw'
import { report, toast } from '../store/ui'
import { Icon } from './Icon'

const REPOSITORY = 'https://github.com/krazyjakee/wobu'

/**
 * The licence Wobu is under, and the notices for everything it is built from.
 *
 * Both texts are read at build time from the files that are the actual licence
 * — `LICENSE` and the generated `THIRD-PARTY-NOTICES.md` — rather than being
 * retyped here. A notice pane that had drifted from the file in the installer
 * would be worse than none: it would be a confident, wrong answer to the one
 * question this pane exists to answer.
 *
 * The notices are a megabyte of licence text, so they arrive on a dynamic
 * import: the chunk is fetched the first time someone asks for them and never
 * in a session that does not. The same file is bundled beside the binary as a
 * resource, so it is readable without launching the app at all.
 */
export function LicencesSection() {
  const [showApp, setShowApp] = useState(false)
  const [notices, setNotices] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function toggleNotices() {
    if (notices !== null) {
      setNotices(null)
      return
    }
    setBusy(true)
    try {
      const module = await import('../../THIRD-PARTY-NOTICES.md?raw')
      setNotices(module.default)
    } catch (e) {
      report(e, 'Could not read the third-party notices')
    } finally {
      setBusy(false)
    }
  }

  function copyNotices() {
    if (notices === null) return
    void navigator.clipboard.writeText(notices).then(
      () => toast('Third-party notices copied.'),
      (e: unknown) => report(e, 'Could not copy the notices'),
    )
  }

  return (
    <section className="set-sec">
      <h3>Licences</h3>
      <p className="set-note">
        Wobu is open source under the MIT licence. Everything it is built from keeps its own — the
        notices below are generated from the two lockfiles, so they describe this build rather than
        an intention, and the same file ships beside the app in the installer.
      </p>

      <div className="set-row">
        <span className="set-label">Wobu</span>
        <span className="set-value">MIT — © 2026 Jake Cattrall</span>
      </div>

      <div className="set-acts">
        <button className="btn-mini" aria-pressed={showApp} onClick={() => setShowApp(!showApp)}>
          <Icon name="search" size="sm" />
          {showApp ? 'Hide licence' : 'Show licence'}
        </button>
        <button
          className="btn-mini"
          aria-pressed={notices !== null}
          disabled={busy}
          onClick={() => void toggleNotices()}
        >
          <Icon name="library" size="sm" />
          {notices !== null
            ? 'Hide third-party notices'
            : busy
              ? 'Loading…'
              : 'Show third-party notices'}
        </button>
        {notices !== null && (
          <button className="btn-mini" onClick={copyNotices}>
            <Icon name="copy" size="sm" />
            Copy
          </button>
        )}
        <button
          className="btn-mini"
          onClick={() => {
            void openUrl(REPOSITORY).catch((e: unknown) => report(e, 'Could not open the browser'))
          }}
        >
          <Icon name="assets" size="sm" />
          Source code
        </button>
      </div>

      {showApp && <pre className="set-log">{appLicence.trim()}</pre>}
      {notices !== null && <pre className="set-log">{notices}</pre>}
    </section>
  )
}
