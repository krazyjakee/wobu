import { call } from './call'
import type { ErrorCode } from './call'
/* ── domain types ─────────────────────────────────────────────────────────── */

/**
 * Where a provider's key came from.
 *
 * `environment` only ever appears in a development build — a repo-root `.env`,
 * or a variable exported into the process. A shipped Wobu reads keys from the
 * OS keychain and from nowhere else.
 */
export type KeySource = 'keychain' | 'environment'

/**
 * Whether this computer has a credential store that answers.
 *
 * `unavailable` is not a failure: a headless Linux box or a session whose login
 * keyring is locked has no store, and the app still runs. What it means is that
 * a key cannot be *saved* here, which is worth saying beside the field rather
 * than discovering when Save fails.
 */
export type KeychainState = 'ready' | 'unavailable'

/**
 * Presence, never value.
 *
 * Keys live in the Rust process and in this machine's keychain; none of them
 * ever crosses the bridge, which is why there is no field here that could carry
 * one. Keys are per *installation*, so a project shared from a drive opens with
 * *your* keys — `project.json` records only which provider and model were
 * chosen, and a collaborator without a key gets `source: null` rather than an
 * error.
 */
export interface KeyStatus {
  provider: string
  /** `null` means no key on this machine. A state, not a failure. */
  source: KeySource | null
  keychain: KeychainState
}

/** Takes a list because a providers pane renders every row at once. */
export const providerKeyStatus = (providers: string[]) =>
  call<KeyStatus[]>('provider_key_status', { providers })

/**
 * The one call that carries key material, and it carries it *inwards*: the user
 * pasted it into a field, so it is already in the webview and the only question
 * is where it goes next. Nothing sends one back.
 *
 * Rejects with `provider.keychain_unavailable` when there is no store to save
 * into — the message says to unlock the login keyring.
 */
export const providerKeySet = (provider: string, key: string) =>
  call<KeyStatus>('provider_key_set', { provider, key })

export interface KeyRemoval {
  /** False when there was nothing stored. A no-op, not a failure. */
  removed: boolean
  /**
   * What the provider resolves to now. On a development build this can still be
   * configured after a successful delete, because the repo-root `.env` answers
   * next — which is the one outcome worth showing rather than assuming.
   */
  status: KeyStatus
}

export const providerKeyDelete = (provider: string) =>
  call<KeyRemoval>('provider_key_delete', { provider })

/* ── the capability probe ─────────────────────────────────────────────────── */

/** What a probe was charged, as the provider reported it. */
export interface ProbeUsage {
  inputTokens: number
  cachedInputTokens: number
  outputTokens: number
}

/**
 * What checking a key found out.
 *
 * A rejected key arrives here as `ok: false`, not as a rejected promise. It is
 * the answer the pane asked for and belongs beside the field that caused it —
 * a toast would put "Anthropic says this key is wrong" somewhere the key is no
 * longer on screen.
 */
export interface ProbeResult {
  provider: string
  /** The model asked about — the adapter's own default when none was passed. */
  model: string
  ok: boolean
  /** One sentence. On success it says what was proved, not just "OK". */
  message: string
  /** `null` when the probe passed. */
  code: ErrorCode | null
  usage: ProbeUsage
}

/**
 * Check a stored key against the provider it belongs to.
 *
 * Cheap by construction rather than by promise: the backend asks for one
 * description and cuts the answer off after a couple of dozen tokens, because
 * everything worth knowing — the key is accepted, the model id resolves, the
 * schema is one this provider will take — is settled before the first sentence
 * is finished. A refused key is never billed at all.
 *
 * Rejects only when the probe could not run: no key on this machine
 * (`provider.no_key`), or a provider this build has no adapter for.
 */
export const providerProbe = (provider: string, model?: string) =>
  call<ProbeResult>('provider_probe', { provider, model })

/* ── machine-local provider settings ────────────────────────────────────── */

/** Installation settings. Deliberately separate from project provider selections. */
export interface MachineSettings {
  comfyuiEndpoint: string
}

export type ComfyEndpointState =
  'connected' | 'unreachable' | 'authentication_required' | 'incompatible'

export interface ComfyEndpointProbe {
  endpoint: string
  state: ComfyEndpointState
  ok: boolean
  message: string
}

/** Read the route stored under this installation's application-data directory. */
export const machineSettings = () => call<MachineSettings>('machine_settings')

/** Validate and persist a ComfyUI route on this machine, never in project.json. */
export const comfyuiEndpointSet = (endpoint: string) =>
  call<MachineSettings>('comfyui_endpoint_set', { endpoint })

/** Probe a draft route without changing which route generation uses. */
export const comfyuiEndpointProbe = (endpoint?: string) =>
  call<ComfyEndpointProbe>('comfyui_endpoint_probe', {
    ...(endpoint === undefined ? {} : { endpoint }),
  })

/* ── the provider selection ───────────────────────────────────────────────── */

/**
 * The three jobs a provider can be chosen for, selected independently.
 *
 * Not one setting: enhancing with Gemini, generating on a ComfyUI running
 * downstairs and meshing through Hunyuan3D is the ordinary combination
 * (`docs/08-providers.md`), and a single provider field cannot express it.
 */
export type Capability = 'text' | 'image' | 'mesh'

/**
 * One capability's entry in `project.json`.
 *
 * All fields are optional because each has a meaningful absence: no `provider`
 * is "nobody has chosen", no `model` is "whatever the adapter's default is",
 * and no `region` means a hosted Hunyuan3D selection is not ready to run.
 */
export interface ProviderSelection {
  provider?: string
  model?: string
  /** Tencent Hunyuan3D only; kept beside the provider because submit and poll must agree. */
  region?: string
}

/**
 * The shared half of the providers pane.
 *
 * This is what `project.json` says, so it is what *everyone* who opens the
 * folder sees — the counterpart to `KeyStatus`, which is what only this machine
 * has. Keeping them as two separate shapes is deliberate: they have different
 * lifetimes, different owners, and merging them into one "provider is ready"
 * flag would erase exactly the distinction a collaborator needs.
 *
 * Keyed loosely because a project written by a newer Wobu may carry a
 * capability this build has never heard of, and the backend round-trips the map
 * rather than parsing it into three fields.
 */
export interface ProviderSelections {
  providers: Record<string, ProviderSelection | undefined>
  /**
   * Whether the *selection* can be changed. Keys are unaffected — they are per
   * installation — so a read-only world is still one you can add a key for.
   */
  readOnly: boolean
}

export const projectProviders = () => call<ProviderSelections>('project_providers')

/**
 * Choose a provider for one capability and write it into `project.json`.
 *
 * Merged rather than replaced on the Rust side, so default params set by another
 * build survive a change of provider. Passing no `model` clears the model, which
 * is how "use the adapter's default" is spelled. Omitting `region` leaves it
 * unchanged; only the explicit Hunyuan region picker sends one.
 *
 * Rejects with `write.read_only` on a read-only folder.
 */
export const projectProviderSelect = (
  capability: Capability,
  provider: string,
  model?: string,
  region?: string,
) =>
  call<ProviderSelections>('project_provider_select', {
    capability,
    provider,
    model,
    ...(region === undefined ? {} : { region }),
  })

/** A provider/model pair after backend defaults have been resolved. */
export interface ActiveModel {
  provider: string
  label: string
  model: string
  contextTokens: number | null
}

export type BackendHealth =
  | { state: 'connected'; externalQueue: number | null }
  | { state: 'unavailable'; detail: string }
  | { state: 'unconfigured'; detail: string }
  | { state: 'unsupported'; detail: string }

export interface StatusBarBackend {
  image: ActiveModel | null
  text: ActiveModel
  health: BackendHealth
}

/** Selected models plus a non-generating reachability check of the image backend. */
export const statusBarBackend = () => call<StatusBarBackend>('status_bar_backend')

/* ── storage and about ────────────────────────────────────────────────────── */
