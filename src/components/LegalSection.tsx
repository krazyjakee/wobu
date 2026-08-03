import { useState } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'
import privacyPolicy from '../../docs/legal/privacy-policy.md?raw'
import termsOfUse from '../../docs/legal/terms.md?raw'
import { report, toast } from '../store/ui'
import { Icon } from './Icon'

const WEB = 'https://github.com/krazyjakee/wobu/blob/main/docs/legal'

const DOCS = {
  privacy: { label: 'privacy policy', text: privacyPolicy, href: `${WEB}/privacy-policy.md` },
  terms: { label: 'terms of use', text: termsOfUse, href: `${WEB}/terms.md` },
} as const

type DocId = keyof typeof DOCS

/**
 * The privacy policy and the terms, in the app.
 *
 * Both texts are read at build time from the files in `docs/legal/` — the same
 * two files the installer drops beside the binary — rather than being retyped
 * here, for the reason `LicencesSection` gives about the notices: a legal pane
 * that had drifted from the document it claims to be would be worse than none.
 *
 * That is also why the summary rows below state only things that cannot go
 * stale — that there is no telemetry, that keys live in the keychain — and the
 * list of network destinations is left to the policy itself. A second,
 * hand-maintained copy of that list in TSX is a promise to keep two things in
 * step, and the one that gets forgotten is the one nobody reads while adding a
 * provider.
 *
 * Placed directly after Providers because that is the pane where a credential
 * is handed over and where content starts leaving the machine, which is the
 * moment the question these documents answer actually occurs to someone.
 */
export function LegalSection() {
  const [open, setOpen] = useState<DocId | null>(null)
  const shown = open === null ? null : DOCS[open]

  function toggle(id: DocId) {
    setOpen((current) => (current === id ? null : id))
  }

  function copy() {
    if (shown === null) return
    void navigator.clipboard.writeText(shown.text).then(
      () => toast(`The ${shown.label} was copied.`),
      (e: unknown) => report(e, `Could not copy the ${shown.label}`),
    )
  }

  function browse(href: string) {
    void openUrl(href).catch((e: unknown) => report(e, 'Could not open the browser'))
  }

  return (
    <section className="set-sec">
      <h3>Legal</h3>
      <p className="set-note">
        Wobu runs no servers of its own. Everything it sends, it sends from this machine straight to
        a provider you configured yourself — the privacy policy lists every destination, what is
        sent to each, and what stays on disk. The same two documents ship beside the app in the
        installer.
      </p>

      <div className="set-row">
        <span className="set-label">Telemetry</span>
        <span className="set-value">None. Nothing is reported back to us, ever.</span>
      </div>
      <div className="set-row">
        <span className="set-label">Your world</span>
        <span className="set-value">Stays in the project folder you chose.</span>
      </div>
      <div className="set-row">
        <span className="set-label">API keys</span>
        <span className="set-value">In this machine&apos;s keychain, never in a project.</span>
      </div>
      <div className="set-row">
        <span className="set-label">Generated work</span>
        <span className="set-value">
          Yours as far as we are concerned; your provider&apos;s terms decide the rest.
        </span>
      </div>

      <div className="set-acts">
        <button
          className={open === 'privacy' ? 'btn-mini is-on' : 'btn-mini'}
          aria-pressed={open === 'privacy'}
          onClick={() => toggle('privacy')}
        >
          <Icon name="lock" size="sm" />
          {open === 'privacy' ? 'Hide privacy policy' : 'Privacy policy'}
        </button>
        <button
          className={open === 'terms' ? 'btn-mini is-on' : 'btn-mini'}
          aria-pressed={open === 'terms'}
          onClick={() => toggle('terms')}
        >
          <Icon name="library" size="sm" />
          {open === 'terms' ? 'Hide terms of use' : 'Terms of use'}
        </button>
        {shown !== null && (
          <>
            <button className="btn-mini" onClick={copy}>
              <Icon name="copy" size="sm" />
              Copy
            </button>
            <button className="btn-mini" onClick={() => browse(shown.href)}>
              <Icon name="link" size="sm" />
              Read on the web
            </button>
          </>
        )}
      </div>

      {shown !== null && (
        <pre className="set-log" aria-label={shown.label}>
          {shown.text.trim()}
        </pre>
      )}
    </section>
  )
}
