import type { WobuError } from './api'

/**
 * Turning a command failure into something a person can read — #127.
 *
 * The Rust side writes its messages for whoever reads the diagnostics log, and
 * `src-tauri/src/error.rs` says so in as many words: every crate's `thiserror`
 * copy is carried across the bridge unchanged. That copy is written in the
 * data model's own vocabulary — "no node with id 01ARZ3NDEKTSV4RRFFQ69G5FAV",
 * "io error at /vol/art/Ashfall.wobu: …", "yaml error: …" — and until this
 * file existed all of it went straight into a toast.
 *
 * So this is the boundary the acceptance criteria of #127 asked for, and it is
 * deliberately the same shape as `failureCopy.ts`, which does the same job for
 * the queue. Two rules run through it:
 *
 * 1. **Never lose a fact the user needs.** Where the backend's sentence is the
 *    only carrier of a path, a count or a version, its wording is kept and a
 *    sentence of guidance is added after it. Only the messages whose specifics
 *    are internal ids — a ULID, a content hash — are replaced outright, and the
 *    technical remainder is still in `detail`, which every toast and banner can
 *    unfold.
 * 2. **One vocabulary.** A record in the world is an *entity* everywhere the
 *    user can see, so a backend sentence that says "node" is rewritten on the
 *    way through rather than being allowed to teach a second word for the same
 *    thing. See the glossary in `docs/guide/reference.md`.
 *
 * It holds no runtime import from `api.ts`, only types, so `errorMessage` can
 * call it without the two files forming a cycle.
 */

/**
 * Codes whose backend sentence says nothing a person can act on, and what to
 * say instead.
 *
 * Written for somebody who has never read a stack trace. Anything that names a
 * place in the app names the whole route to it, because "check your settings"
 * is not an instruction — the same rule `failureCopy.ts` follows, so the two
 * surfaces cannot end up giving different advice for one cause.
 */
const REPLACEMENTS: Record<string, string> = {
  'project.none_open': 'No project is open.',
  'transfer.same_project':
    'The project you are copying from is the one that is already open. Choose a different project as the source.',

  'node.not_found':
    'That entity is not in this project any more. It may have been deleted here, or by somebody else who shares this folder.',
  'asset.not_found':
    'That image is not in this project any more, or it is no longer attached to this entity. The panel you are looking at is out of date — reopen it to see what is really there.',
  'asset.in_use':
    'This image is still attached to at least one entity, or is being used as a cover. Detach it everywhere it is used, then delete it.',

  'write.read_only':
    'The project folder is read-only, so nothing can be saved into it. Change the folder’s permissions, or copy the project somewhere you can write to, and open it again.',
  'share.unmounted':
    'Wobu cannot reach the project folder. If it is on a network share or a removable drive, reconnect it — Wobu keeps trying and carries on where it left off.',
  'sync.unreachable':
    'Wobu could not reach the other machine. Check that Wobu is open there and both machines are online, then try again.',
  'io.failed':
    'Wobu could not read or write a file in the project folder. Check there is free disk space and that the folder is still writable, then try again.',

  // Kept word-for-word in step with `failureCopy.ts`: the same cause must not
  // give one answer when a job hits it and a different one when a command does.
  'provider.no_key':
    'Wobu has no key for this provider yet. Open Settings → Providers and models and add one, then try again.',
  'provider.bad_key':
    'The provider rejected the key Wobu is using. Open Settings → Providers and models and paste the key again — a key that has been revoked or copied with a stray space fails exactly like this.',
  'provider.keychain_unavailable':
    'Wobu could not save the key to either the operating-system credential store or its private local store. Check free disk space and permissions for Wobu’s application-data directory, then try again.',
  'provider.clock_skew':
    'Your computer’s clock is too far from real time for the provider to accept the request. Turn on automatic time in your system settings, then try again.',
  'provider.billing_required':
    'The provider will not run this model without billing enabled on the account. Enable it with the provider, then try again.',
  'provider.rate_limited':
    'The provider is turning requests away because this account has sent too many too quickly. Wait a moment and try again; running fewer jobs at once, in Settings → Providers and models, stops it recurring.',
  'provider.unavailable':
    'Wobu could not reach the provider. If it runs on this machine, check it is still running; if it is a remote service, check your connection. Then try again.',
  'provider.bad_response':
    'The provider answered, but not with anything Wobu could use. Trying again usually works.',
  'provider.context_too_long':
    'What Wobu sent the provider was longer than the model accepts. Shorten the notes, or mute a layer or two in the inspector, then try again.',
  internal:
    'Something went wrong inside Wobu itself, rather than in anything you did. Settings → Diagnostics has the log, if you would like to report it.',
}

/**
 * Codes whose backend sentence carries the only copy of a path, a count or a
 * version number — kept, with the missing half of the answer added after it.
 */
const GUIDANCE: Record<string, string> = {
  'project.not_a_project':
    'Choose the folder that has project.json inside it, or make a new project here.',
  'project.already_exists': 'Choose an empty folder, or open the project that is already there.',
  'project.schema_too_new':
    'Update Wobu to open it. An older build would open it and quietly drop everything it does not understand.',
  'node.malformed':
    'Wobu has left the file exactly as it is rather than write over it. Open it in a text editor and check the block of settings at the top — a hand edit or a half-finished sync is the usual cause.',
}

/**
 * The backend's own wording, made to read like the rest of the app.
 *
 * Sentence case and a full stop, because these are assembled into `report`'s
 * `prefix — message` form and a lower-case fragment there reads like a
 * truncation. Then the one vocabulary substitution: "node" is the word the
 * data model uses and "entity" is the word the user is taught, and a message
 * that leaks the first undoes every label that says the second.
 */
export function humaniseBackendMessage(message: string): string {
  const swapped = message
    // A path segment — `nodes/species/vashk.md` — is a real folder name and is
    // left alone; only the standalone English word is rewritten.
    .replace(/(^|[^\w/`])nodes([^\w/]|$)/g, '$1entities$2')
    .replace(/(^|[^\w/`])node([^\w/]|$)/g, '$1entity$2')
    // "a node" becomes "an entity", not "a entity". The article has to move
    // with the noun or the sentence announces that a machine rewrote it.
    .replace(/\ba node\b/g, 'an entity')
    .replace(/\bA node\b/g, 'An entity')
    .replace(/(^|[^\w])a entity\b/g, '$1an entity')
    .replace(/(^|[^\w])A entity\b/g, '$1An entity')
  const trimmed = swapped.trim()
  if (!trimmed) return trimmed
  const capitalised = trimmed[0]!.toLocaleUpperCase() + trimmed.slice(1)
  return /[.!?…]$/.test(capitalised) ? capitalised : `${capitalised}.`
}

/**
 * A save that lost a race, said in full.
 *
 * Rebuilt here rather than humanised, because `conflictPath` arrives as its own
 * field: the whole fact is available without parsing it back out of a sentence,
 * and the backend's version of that sentence says "node".
 */
function conflictText(error: WobuError): string {
  const path = error.conflictPath
  const parked = path
    ? `Your version was saved alongside theirs, as ${path}, rather than overwriting anything.`
    : 'Your version was saved alongside theirs rather than overwriting anything.'
  return `Somebody else changed this entity while you were editing it. ${parked} Open both and decide which text to keep.`
}

/**
 * What to show a person for a failed command.
 *
 * `code` is matched first so that a build meeting a code it has never heard of
 * degrades to the backend's own wording rather than to silence.
 */
export function plainError(error: WobuError): string {
  if (error.code === 'write.conflict') return conflictText(error)

  const replacement = REPLACEMENTS[error.code as string]
  if (replacement) return replacement

  const humanised = humaniseBackendMessage(error.message)
  const guidance = GUIDANCE[error.code as string]
  return guidance ? `${humanised} ${guidance}` : humanised
}
