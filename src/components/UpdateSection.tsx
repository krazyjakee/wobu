import { useState } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { relaunch } from '@tauri-apps/plugin-process'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { errorMessage } from '../lib/api'
import { report } from '../store/ui'
import { Icon } from './Icon'

const RELEASES = 'https://github.com/krazyjakee/wobu/releases'

/**
 * What the pane is showing right now.
 *
 * A discriminated union rather than four booleans because the states genuinely
 * exclude one another — "checking" and "downloading" both being true is not a
 * screen anyone should be able to reach — and because `failed` carries its own
 * message. The failure stays *here*, next to the button that caused it, instead
 * of only surfacing as a toast that a user who has looked away will miss.
 */
type State =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'current' }
  | { kind: 'found'; update: Update }
  | { kind: 'downloading'; version: string; done: number; total: number | null }
  | { kind: 'installed'; version: string }
  | { kind: 'failed'; message: string }

function percent(done: number, total: number | null): string {
  if (total === null || total <= 0) return `${Math.round(done / 1_000_000)} MB`
  return `${Math.min(100, Math.round((done / total) * 100))}%`
}

/**
 * Checking for, and installing, a new version of Wobu.
 *
 * Deliberately manual. Nothing here runs at startup and nothing polls: opening
 * Wobu is not consent to contact GitHub, and a local-first app that phoned home
 * unasked would contradict the promise the Legal pane makes three sections up.
 * The button is the request.
 *
 * The security story is the signature, not the transport. `tauri.conf.json`
 * carries the public half of an offline keypair; the private half exists only
 * as a GitHub Actions secret. The plugin verifies the downloaded bundle against
 * that key before it writes anything, so a replaced `latest.json`, a hostile
 * mirror or a compromised release asset produces a refusal rather than an
 * install. That is a narrower claim than code signing — first installs are
 * still unsigned downloads and Gatekeeper/SmartScreen still warn — and the note
 * below says so rather than implying the warning has gone away.
 *
 * On Linux only the AppImage can replace itself; a `.deb` is owned by the
 * package manager and the plugin will refuse it. That refusal is reported as
 * the ordinary "download it again" path rather than as a fault, because for
 * that user it is not one.
 */
export function UpdateSection() {
  const [state, setState] = useState<State>({ kind: 'idle' })
  const busy = state.kind === 'checking' || state.kind === 'downloading'

  async function look() {
    setState({ kind: 'checking' })
    try {
      const update = await check()
      setState(update === null ? { kind: 'current' } : { kind: 'found', update })
    } catch (e: unknown) {
      setState({ kind: 'failed', message: errorMessage(e) })
    }
  }

  async function install(update: Update) {
    const version = update.version
    setState({ kind: 'downloading', version, done: 0, total: null })
    let done = 0
    let total: number | null = null
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === 'Started') {
          total = event.data.contentLength ?? null
        } else if (event.event === 'Progress') {
          done += event.data.chunkLength
        }
        setState({ kind: 'downloading', version, done, total })
      })
      setState({ kind: 'installed', version })
    } catch (e: unknown) {
      setState({ kind: 'failed', message: errorMessage(e) })
    }
  }

  function restart() {
    // Only reached after a successful install, so a failure here is the
    // relaunch itself: the new version is already on disk either way, which is
    // what the message says rather than implying the update was lost.
    void relaunch().catch((e: unknown) => report(e, 'Could not restart — quit and reopen Wobu'))
  }

  function browse() {
    void openUrl(RELEASES).catch((e: unknown) => report(e, 'Could not open the browser'))
  }

  return (
    <section className="set-sec">
      <h3>Updates</h3>
      <p className="set-note">
        Wobu checks for a new version only when you ask it to. Nothing is downloaded until you say
        so, and every update is verified against a signing key built into this app before it is
        installed — a release that was not signed by the Wobu project is refused, not installed.
      </p>

      <div className="set-row">
        <span className="set-label">Status</span>
        <span className="set-value">
          {state.kind === 'idle' && 'Not checked this session.'}
          {state.kind === 'checking' && 'Asking GitHub for the latest release…'}
          {state.kind === 'current' && 'You are on the latest release.'}
          {state.kind === 'found' && `Wobu ${state.update.version} is available.`}
          {state.kind === 'downloading' &&
            `Downloading ${state.version} — ${percent(state.done, state.total)}`}
          {state.kind === 'installed' &&
            `Wobu ${state.version} is installed. It starts when Wobu next opens.`}
          {state.kind === 'failed' && state.message}
        </span>
      </div>

      {state.kind === 'found' && state.update.body !== undefined && state.update.body !== '' && (
        <pre className="set-log" aria-label={`release notes for ${state.update.version}`}>
          {state.update.body.trim()}
        </pre>
      )}

      <div className="set-acts">
        <button className="btn-mini" onClick={() => void look()} disabled={busy}>
          <Icon name="refresh" size="sm" />
          {state.kind === 'checking' ? 'Checking…' : 'Check for updates'}
        </button>
        {state.kind === 'found' && (
          <button className="btn-mini" onClick={() => void install(state.update)}>
            <Icon name="spark" size="sm" />
            Download and install {state.update.version}
          </button>
        )}
        {state.kind === 'installed' && (
          <button className="btn-mini" onClick={restart}>
            <Icon name="check" size="sm" />
            Restart now
          </button>
        )}
        <button className="btn-mini" onClick={browse}>
          <Icon name="link" size="sm" />
          All releases
        </button>
      </div>

      <p className="set-note">
        Your projects are ordinary folders and are never stored inside the application, so an update
        cannot touch them. Installs from a Linux <code>.deb</code> or a Windows <code>.msi</code>{' '}
        are managed by their installer rather than by Wobu — download the new one from the releases
        page. First installs are still unsigned: the platform will warn about an unknown publisher,
        and the release guide explains what to check before continuing.
      </p>
    </section>
  )
}
