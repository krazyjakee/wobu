import type { JobFailure, JobKind } from './api'

/**
 * Turning a queue failure into something a person can act on.
 *
 * Every string a failed job puts in front of the user comes from here, for one
 * reason: the messages the backend produces are aimed at whoever reads the log.
 * `wobu_imagine::Error::Unsupported` says so in as many words, and the failure
 * that opened #142 —
 * `this backend cannot honour the request: invalid_request: Image delivery mode
 * is not supported.` — is exactly what that reads like when it reaches somebody
 * who was drawing a character. The provider's own sentence is still shown, as
 * the *reason*; it is never the whole of what is said.
 *
 * The split is deliberate: `title` is what happened, `guidance` is what to do
 * next, `reason` is the backend's wording, `detail` is the technical remainder
 * that starts folded. A failure with no guidance is a failure the user is
 * expected to solve by staring at it.
 */

/** What the user set going, in their words rather than the queue's. */
export function jobKindLabel(kind: JobKind): string {
  switch (kind) {
    case 'generate':
      return 'Image generation'
    case 'mesh':
      return '3D reconstruction'
    case 'enhance':
      return 'Description enhance'
    case 'train_lora':
      return 'LoRA training'
    case 'thumbnail':
      return 'Thumbnail'
    default:
      return 'Job'
  }
}

/**
 * What was spent on an attempt that produced nothing, or `null` when the
 * failure cost nothing at all.
 *
 * `unknown` is not rounded down to "free". The queue treats it as charged when
 * it decides whether to retry (see `wobu_jobs::Billed`), and telling somebody
 * their money is safe when the backend never said so is the one lie this
 * surface must not tell.
 */
export function chargeLine(failure: JobFailure): string | null {
  if (failure.billed === 'nothing') return null
  const cost = failure.costNote?.trim()
  if (failure.billed === 'charged') {
    return cost
      ? `You were charged for this attempt and got nothing back: ${cost}.`
      : 'You were charged for this attempt and got nothing back. The backend did not say how much.'
  }
  return cost
    ? `This attempt may have been charged for: ${cost}.`
    : 'The backend did not say whether this attempt was charged for. Treat it as spent.'
}

/** True when the user's money is, or may be, gone. Drives the loudest surface. */
export function costsMoney(failure: JobFailure): boolean {
  return failure.billed !== 'nothing'
}

/**
 * What to do next, per error code.
 *
 * Written for somebody who has never read a stack trace and does not know what
 * a provider is. Anything that names a place in the app names the whole route
 * to it, because "check your settings" is not an instruction.
 */
const GUIDANCE: Record<string, string> = {
  'provider.no_key':
    'Wobu has no key for this backend yet. Open Settings → Providers and add one, then start the job again.',
  'provider.bad_key':
    'The backend rejected the key Wobu is using. Open Settings → Providers and paste the key again — a key that has been revoked or copied with a stray space fails exactly like this.',
  'provider.keychain_unavailable':
    'Your operating system would not open its keychain, so the saved key could not be read. Unlock the keychain — logging out and back in is usually enough — then start the job again.',
  'provider.clock_skew':
    "Your computer's clock is too far from real time for the backend to accept the request. Turn on automatic time in your system settings, then start the job again.",
  'provider.billing_required':
    'The backend will not run this model without billing enabled on the account. Enable it with the provider, then start the job again.',
  'provider.unavailable':
    'Wobu could not reach the backend. If it runs on this machine, check it is still running; if it is a remote service, check your connection. Then start the job again.',
  'provider.bad_response':
    'The backend answered, but not with anything Wobu could use. Starting the job again with a new seed usually works — if it keeps happening, the prompt may be being refused rather than failing.',
  'provider.context_too_long':
    'The prompt sent to the backend was longer than the model accepts. Shorten the description, or mute a layer or two in the Inspector, then start the job again.',
  'billing.ceiling_exceeded':
    "This project's spending ceiling has been reached, so Wobu stopped before spending more. Raise the ceiling in Settings → Providers if you meant to continue.",
  'write.read_only':
    'The result could not be written because the project folder is read-only. Fix the folder permissions, or copy the project somewhere writable, then start the job again.',
  'share.unmounted':
    'The project folder went away while the job was running, so nothing could be saved. Reconnect the drive or share and start the job again.',
  'io.failed':
    'Wobu could not write the result into the project folder. Check there is free disk space and that the folder is still writable, then start the job again.',
  'node.not_found':
    'The entity this job was for no longer exists, so there was nowhere to put the result. Nothing is wrong with the backend.',
  internal:
    'This is a fault in Wobu rather than in your prompt, your key or your machine. Nothing you can change will fix it. Please report it with the details below.',
}

/**
 * The class of failure #142 was filed about: Wobu asked for something the
 * backend does not accept.
 *
 * It arrives as `internal` with the adapter's log sentence for a message, and
 * the generic `internal` guidance is nearly right — but "report it" is a poor
 * answer when the fix has usually already shipped. The concrete example, the
 * Gemini image delivery mode, was corrected in
 * `src-tauri/crates/wobu-imagine/src/gemini/wire.rs`, so anybody meeting it now
 * is running a build from before that, and updating is the actual fix.
 */
function rejectedRequest(failure: JobFailure): string | null {
  if (failure.code !== 'internal') return null
  const message = failure.message.toLowerCase()
  if (!message.includes('cannot honour the request')) return null
  return 'Wobu asked the backend for an option it does not accept, so nothing was generated. This is a fault in Wobu, not in your prompt. Check for a Wobu update first — several of these have already been fixed — and report it with the details below if you are up to date.'
}

/** A rate limit is the one code whose answer depends on a number in the payload. */
function rateLimited(failure: JobFailure): string {
  const seconds = failure.retryAfter ? Math.ceil(failure.retryAfter / 1_000) : null
  const wait = seconds
    ? `The backend asked for ${seconds} second${seconds === 1 ? '' : 's'} before the next attempt.`
    : 'The backend did not say how long to wait.'
  return `The backend is turning requests away because this account has sent too many too quickly. ${wait} Running fewer jobs at once, in Settings → Providers, stops it recurring.`
}

export function failureGuidance(failure: JobFailure): string {
  const rejected = rejectedRequest(failure)
  if (rejected) return rejected
  if (failure.code === 'provider.rate_limited') return rateLimited(failure)
  const known = GUIDANCE[failure.code]
  if (known) return known
  return failure.retryable
    ? 'Wobu does not have specific advice for this one. Starting the job again is worth a try; the backend’s own words are below.'
    : 'Wobu does not have specific advice for this one, and starting the job again would fail the same way. The backend’s own words are below.'
}

/**
 * The whole of what a failed job says, ready to be rendered.
 *
 * `title` deliberately carries the money. A user scanning a list of headlines
 * must not have to open one to find out that it cost them something.
 */
export interface FailureNotice {
  title: string
  guidance: string
  reason: string
  detail?: string
  charge: string | null
}

export function failureNotice(kind: JobKind, failure: JobFailure): FailureNotice {
  const charge = chargeLine(failure)
  const paid = costsMoney(failure)
  return {
    title: paid
      ? `${jobKindLabel(kind)} failed after it was billed`
      : `${jobKindLabel(kind)} failed`,
    guidance: failureGuidance(failure),
    reason: failure.message,
    detail: failure.detail,
    charge,
  }
}
