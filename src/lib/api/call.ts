import { invoke } from '@tauri-apps/api/core'
import { plainError } from '../errorCopy'
/* ── domain types ─────────────────────────────────────────────────────────── */

/** True when running inside the Tauri webview (as opposed to a bare `vite dev`). */
export const isTauri = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/* ── errors ───────────────────────────────────────────────────────────────── */

/**
 * The stable codes `src-tauri/src/error.rs` emits. Renaming one means renaming
 * it on both sides at once — there is a Rust test pinning every string here.
 *
 * Left open with `(string & {})` on purpose: the provider codes exist in Rust
 * before the provider crates do, and a code this build has never heard of must
 * degrade to "unknown error, show the message" rather than fail to type-check.
 */
export type ErrorCode =
  | 'project.not_a_project'
  | 'project.already_exists'
  | 'project.schema_too_new'
  | 'project.none_open'
  | 'transfer.same_project'
  | 'node.not_found'
  | 'node.malformed'
  | 'node.invalid'
  | 'write.conflict'
  | 'write.read_only'
  | 'asset.not_an_image'
  | 'asset.not_found'
  | 'asset.in_use'
  | 'share.unmounted'
  | 'provider.no_key'
  | 'provider.keychain_unavailable'
  | 'provider.bad_key'
  | 'provider.clock_skew'
  | 'provider.billing_required'
  | 'provider.rate_limited'
  | 'provider.unavailable'
  | 'provider.bad_response'
  | 'provider.context_too_long'
  | 'billing.ceiling_exceeded'
  | 'io.failed'
  | 'cancelled'
  | 'internal'
  | (string & {})

/** Exactly what a failing command rejects with. */
export interface WobuError {
  code: ErrorCode
  message: string
  /** Technical remainder — the OS's own wording, a parser position. */
  detail?: string
  retryable: boolean
  /** Only on `write.conflict`: where our version was parked. */
  conflictPath?: string
}

export function isWobuError(e: unknown): e is WobuError {
  return (
    !!e &&
    typeof e === 'object' &&
    typeof (e as WobuError).code === 'string' &&
    typeof (e as WobuError).message === 'string'
  )
}

/** The code, or `null` for anything that did not come from a command. */
export function errorCode(e: unknown): ErrorCode | null {
  return isWobuError(e) ? e.code : null
}

/**
 * Where an error belongs on screen.
 *
 * A banner is persistent and occupies real estate, so it is reserved for the
 * two things that make the *whole workspace* untrustworthy until resolved:
 * the project folder went away, or it turned read-only underneath us.
 * Everything else — a rejected save, a bad name, a failed thumbnail — is a
 * toast, because the user already knows which action produced it and the rest
 * of the app still works. Getting this wrong in the generous direction is
 * worse: a banner that appears for routine failures is one the user learns to
 * ignore.
 *
 * Both banner codes are specifically *mid-session* conditions. Open-time
 * failures are not here on purpose: `project.not_a_project` and
 * `project.schema_too_new` can only happen while the Launcher is on screen,
 * which has its own inline error slot and no workspace to put a banner above.
 * A read-only folder detected at open does get a banner, but not through here:
 * the Workspace raises `project.read_only` once when the project opens (see
 * `src/lib/readOnly.ts`), because nothing failed — that is the state of the
 * folder rather than the outcome of a command. `write.read_only` reaching
 * *here* means a folder that started writable changed underneath the session,
 * which is a failure and needs saying in its own words.
 */
export type Surface = 'banner' | 'toast' | 'silent'

export function errorSurface(e: unknown): Surface {
  switch (errorCode(e)) {
    case 'share.unmounted':
    case 'write.read_only':
      return 'banner'
    // Not a failure. The user asked for this to stop, and telling them it
    // stopped is the app arguing with them about something they just did.
    case 'cancelled':
      return 'silent'
    default:
      return 'toast'
  }
}

export function isRetryable(e: unknown): boolean {
  return isWobuError(e) && e.retryable
}

/**
 * What to put in front of a person when a command fails.
 *
 * Normalises the assorted things a Rust command error can arrive as, and — for
 * anything that came from the bridge — hands it to `lib/errorCopy.ts` to be
 * said in the app's own words rather than the data model's. That translation
 * lives here, at the one function every error surface already went through, so
 * no call site has to remember to ask for it (#127).
 *
 * The untranslated sentence is not lost: it is in the diagnostics log, written
 * by `WobuError::new` at the moment the error was made.
 */
export function errorMessage(e: unknown): string {
  if (typeof e === 'string') return e
  if (isWobuError(e)) return plainError(e)
  if (e instanceof Error) return e.message
  if (e && typeof e === 'object') {
    const m = (e as { message?: unknown }).message
    if (typeof m === 'string') return m
    try {
      return JSON.stringify(e)
    } catch {
      return String(e)
    }
  }
  return String(e)
}

export function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    return Promise.reject(
      new Error(
        `Not running inside Tauri — the "${cmd}" command is unavailable. Launch with \`npm run tauri dev\`.`,
      ),
    )
  }
  return invoke<T>(cmd, args)
}

/* ── commands ─────────────────────────────────────────────────────────────── */
