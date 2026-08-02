import { useCallback, useEffect, useRef, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { openUrl } from '@tauri-apps/plugin-opener'
import type {
  AboutInfo,
  Capability,
  ComfyEndpointProbe,
  IndexInfo,
  KeyStatus,
  LogInfo,
  LogLevel,
  ProbeResult,
  ProjectSummary,
  ProviderSelection,
} from '../lib/api'
import {
  LOG_LEVELS,
  aboutInfo,
  comfyuiEndpointProbe,
  comfyuiEndpointSet,
  errorMessage,
  indexInfo,
  indexRebuild,
  logInfo,
  logReveal,
  logSetLevel,
  logTail,
  machineSettings,
  providerProbe,
} from '../lib/api'
import {
  invalidateWorld,
  useDeleteProviderKey,
  useProviderKeys,
  useProviderSelections,
  useSelectProvider,
  useSetProviderKey,
} from '../lib/queries'
import {
  AUTOSAVE_DEFAULT,
  AUTOSAVE_MAX,
  AUTOSAVE_MIN,
  SCALE_MAX,
  SCALE_MIN,
  SCALE_STEP,
  useSettings,
} from '../store/settings'
import { report, toast } from '../store/ui'
import { ConfirmSheet } from './ConfirmSheet'
import { Icon } from './Icon'
import { WikiExportSection } from './WikiExportSection'

/**
 * The Settings surface.
 *
 * Sections are independent and each owns its own loading, so a new pane drops
 * in as one more `<section className="set-sec">` rather than a rewrite. Nothing
 * here is stubbed: a control that looks configurable but is not is worse than
 * an honest absence, which is why there is no theme switch — see Appearance.
 */
export function Settings({ project }: { project?: ProjectSummary }) {
  return (
    <div className="settings-mode">
      <div className="settings">
        <h2>Settings</h2>
        <Providers />
        <Storage />
        <EditorPrefs />
        <Appearance />
        {project && <WikiExportSection project={project} />}
        <Diagnostics />
        <About />
      </div>
    </div>
  )
}

/* ── providers ────────────────────────────────────────────────────────────── */

/**
 * One credential a provider needs before it will answer.
 *
 * A list rather than a string because Tencent's is a SecretId/SecretKey *pair*
 * signed together, not a bearer token — `keys.rs` registers it as two keychain
 * entries for that reason, and a pane that assumed one field per provider would
 * have nowhere to put the second.
 */
interface Credential {
  /** The `wobu/<id>` keychain entry, and the id every command here takes. */
  id: string
  label: string
}

interface ProviderDef {
  /** The id `project.json` carries. */
  id: string
  label: string
  /** Empty for a backend that authenticates nothing. See ComfyUI. */
  credentials: Credential[]
  /** Where the user goes to get one. */
  where?: string
  /** Whether this build has an adapter that can check a key for it. */
  checkable?: boolean
  /** Said instead of a key field, for a provider that needs none. */
  instead?: string
  /** Material capability or licence difference a user must see before selecting it. */
  tierNote?: string
  /** Whether credentials need the Tencent account-safety setup shown before the fields. */
  hunyuanOnboarding?: boolean
}

const ANTHROPIC: ProviderDef = {
  id: 'anthropic',
  label: 'Anthropic',
  credentials: [{ id: 'anthropic', label: 'API key' }],
  where: 'the Claude Console',
  checkable: true,
}

const GEMINI: ProviderDef = {
  id: 'gemini',
  label: 'Gemini',
  credentials: [{ id: 'gemini', label: 'API key' }],
  where: 'Google AI Studio',
  checkable: true,
}

const COMFYUI: ProviderDef = {
  id: 'comfyui',
  label: 'ComfyUI',
  credentials: [],
  instead:
    'ComfyUI needs no key — it is a server you run yourself. Its machine-local address is ' +
    'configured and checked below instead of being written into the shared project.',
}

const HUNYUAN3D: ProviderDef = {
  id: 'hunyuan3d',
  label: 'Tencent Hunyuan3D',
  credentials: [
    { id: 'tencent-secret-id', label: 'Secret ID' },
    { id: 'tencent-secret-key', label: 'Secret key' },
  ],
  where: 'a Tencent CAM sub-account',
  hunyuanOnboarding: true,
}

const HUNYUAN_REGIONS = [
  { id: 'ap-singapore', label: 'Singapore (Asia-Pacific)' },
  { id: 'na-siliconvalley', label: 'Silicon Valley (North America)' },
  { id: 'eu-frankfurt', label: 'Frankfurt (Europe)' },
] as const

const COMFYUI_MESH: ProviderDef = {
  id: 'comfyui',
  label: 'Local 2.1 (ComfyUI)',
  credentials: [],
  tierNote:
    'Explicit local tier — never an automatic fallback when a Tencent key is absent. It uses the ' +
    'older, lower-quality Hunyuan3D 2.1 shape model: one front image, geometry-only output and at ' +
    'least 10 GB VRAM, with no per-job fee and image data staying on your ComfyUI machine. Wobu ' +
    'does not install its weights or nodes. Tencent’s model licence excludes the EU, UK and South ' +
    'Korea, so check that you are permitted to use it where you are.',
}

interface CapabilityDef {
  capability: Capability
  label: string
  /** What in the app uses this choice. */
  used: string
  icon: string
  providers: ProviderDef[]
  /** Whether choosing a model here means anything to this build. */
  model: boolean
  /**
   * What runs when `project.json` names nothing.
   *
   * Not a display default: `enhance.rs` really does fall back to Anthropic, so
   * a pane that showed "nothing chosen" and left it there would be describing a
   * project that spends money at a provider it never mentioned. Every world
   * made before this pane existed is in exactly that state.
   */
  fallback?: ProviderDef
  /** What the current product surface does with this capability. */
  activeNote?: string
}

/**
 * Three capabilities, chosen separately.
 *
 * Enhancing with Gemini, generating on a ComfyUI running under your desk and
 * meshing through Hunyuan3D is the ordinary combination rather than the exotic
 * one, and a single "provider" setting could not express it at all.
 */
const CAPABILITIES: CapabilityDef[] = [
  {
    capability: 'text',
    label: 'Text',
    used: 'Enhance',
    icon: 'spark',
    providers: [ANTHROPIC, GEMINI],
    model: true,
    fallback: ANTHROPIC,
  },
  {
    capability: 'image',
    label: 'Image',
    used: 'Generate',
    icon: 'image',
    providers: [COMFYUI, GEMINI],
    model: false,
    activeNote:
      'Generate and Forge use this choice for entity images, variant grids and scene compositions.',
  },
  {
    capability: 'mesh',
    label: 'Mesh',
    used: 'Concept 3D',
    icon: 'cube',
    providers: [HUNYUAN3D, COMFYUI_MESH],
    model: false,
    activeNote:
      'Concept 3D can view and export completed GLBs from the project asset library. Starting reconstruction from this UI is not available yet.',
  },
]

/**
 * Every provider that has a key, once each.
 *
 * Once, because a key is not per capability: Gemini writes text and makes
 * pictures on the same credential, and listing it twice would ask the user for
 * two keys and store one. This ordering is the order the key rows appear in.
 */
const KEYED: ProviderDef[] = [ANTHROPIC, GEMINI, HUNYUAN3D, COMFYUI]

/**
 * Module-level so the array identity is stable — it is part of the React Query
 * key, and a fresh array every render would refetch on every render.
 */
const CREDENTIAL_IDS: string[] = KEYED.flatMap((p) => p.credentials.map((c) => c.id))

/**
 * Providers and keys — and above all, which of the two is shared.
 *
 * The pane is two bands rather than one list of providers, and that is the
 * whole design rather than a layout choice. A project's *selection* lives in
 * `project.json` and goes wherever the folder goes: open a world from a shared
 * drive and you are looking at somebody else's decision. A *key* is per
 * installation, sits in this machine's keychain, and goes nowhere at all. Those
 * two facts are what BYOK means, they are the two facts users get wrong, and a
 * sentence explaining them is worth much less than a layout in which they are
 * obviously separate things — so the pane keeps them apart, labels each band
 * with what it is, and never repeats a key row inside a capability.
 *
 * See `docs/08-providers.md`.
 */
function Providers() {
  const selections = useProviderSelections()
  const keys = useProviderKeys(CREDENTIAL_IDS)
  const [focus, setFocus] = useState<string | null>(null)
  // Stable identity: it is an effect dependency in every credential row, and a
  // fresh closure per render would re-run all of them on every render.
  const clearFocus = useCallback(() => setFocus(null), [])

  const statuses = keys.data
  const selected = selections.data

  // Both are one round trip on mount and neither is worth a spinner; the
  // sections around this one take the same line.
  if (!statuses || !selected) return null
  const chosen = selected.providers

  const status = (id: string): KeyStatus | undefined => statuses.find((s) => s.provider === id)

  /** Whether every credential this provider needs is present on this machine. */
  const configured = (provider: ProviderDef) =>
    provider.credentials.every((c) => status(c.id)?.source)

  // A property of the machine rather than of a provider, which is why one line
  // covers the whole band. `keys.rs` reports it per provider because that is
  // where it has to be said, not because it varies.
  const keychainDown = statuses.some((s) => s.keychain === 'unavailable')

  return (
    <section className="set-sec">
      <h3>Providers and models</h3>
      <p className="set-note">
        Two different things live here and they belong to different people. What this project uses
        is written into the project folder and travels with it, so opening a shared world shows you
        the choices whoever built it made. Your keys never travel: they stay in this
        computer&rsquo;s keychain, and everyone who opens the same world runs it on their own.
      </p>

      <div className="prov-band prov-band-shared">
        <div className="prov-band-head">
          <Icon name="share" size="sm" />
          <span className="prov-band-title">What this project uses</span>
          <span className="badge">shared</span>
        </div>
        <p className="set-note">
          In <code>project.json</code>, beside the world itself — never a key.
          {selected.readOnly &&
            ' This folder is read-only, so the choices below cannot be changed from here.'}
        </p>
        {CAPABILITIES.map((def) => (
          <CapabilityRow
            key={def.capability}
            def={def}
            selection={chosen[def.capability] ?? {}}
            readOnly={selected.readOnly}
            configured={configured}
            onAddKey={setFocus}
          />
        ))}
      </div>

      <div className="prov-band prov-band-local">
        <div className="prov-band-head">
          <Icon name="lock" size="sm" />
          <span className="prov-band-title">Keys on this computer</span>
          <span className="badge">local</span>
        </div>
        <p className="set-note">
          Listed once each, because a key is not per capability — the same Gemini key writes text
          and makes pictures. The ComfyUI route belongs here for the same reason: it describes this
          computer, not the shared world. Nothing here is written into the project folder, and
          nothing you paste is ever sent back to this window.
        </p>
        <ComfyEndpointSettings />
        {keychainDown && (
          <p className="prov-alert">
            This computer&rsquo;s credential store is not answering. On Linux that usually means the
            login keyring is locked; a headless session has none at all. Keys cannot be saved until
            it is unlocked — a key already in the environment still works.
          </p>
        )}
        {KEYED.map((provider) => (
          <ProviderKeys
            key={provider.id}
            provider={provider}
            statuses={statuses}
            model={chosen.text?.provider === provider.id ? chosen.text?.model : undefined}
            keychainDown={keychainDown}
            focus={focus}
            onFocused={clearFocus}
          />
        ))}
      </div>
    </section>
  )
}

/**
 * One capability's choice, and what is missing before it can run.
 *
 * The "selected but no key here" state is rendered inline rather than as a
 * failure, because it is the *expected* state for a collaborator on day one:
 * they opened somebody else's world, and the selection came with it. Saying so
 * next to the choice, with a way straight to the field that fixes it, is the
 * alternative to finding out at generate time on a call that has already been
 * queued.
 */
function CapabilityRow({
  def,
  selection,
  readOnly,
  configured,
  onAddKey,
}: {
  def: CapabilityDef
  selection: ProviderSelection
  readOnly: boolean
  configured: (provider: ProviderDef) => boolean
  onAddKey: (credentialId: string) => void
}) {
  const select = useSelectProvider()
  const chosen = def.providers.find((p) => p.id === selection.provider)
  const missing = chosen && !configured(chosen)
  const hunyuanRegion = HUNYUAN_REGIONS.find((region) => region.id === selection.region)?.id ?? ''

  return (
    // Grouped and named because the same provider appears under more than one
    // capability — Gemini writes text and makes pictures — so "the Gemini
    // button" is ambiguous to a screen reader for exactly the reason it is
    // ambiguous to a reader.
    <div className="prov-cap" role="group" aria-label={`${def.label} — ${def.used}`}>
      <div className="prov-cap-head">
        <Icon name={def.icon} size="sm" />
        <span className="prov-cap-title">{def.label}</span>
        <span className="prov-cap-used">{def.used}</span>
      </div>

      <div className="set-levels">
        {def.providers.map((p) => (
          <button
            key={p.id}
            className={p.id === selection.provider ? 'btn-mini is-on' : 'btn-mini'}
            aria-pressed={p.id === selection.provider}
            disabled={readOnly || select.isPending}
            // The model is deliberately not carried across. Model ids belong to
            // one vendor — `claude-sonnet-5` handed to Gemini is a request that
            // fails for a reason nothing on screen explains — so changing the
            // provider drops back to that adapter's own default.
            onClick={() => select.mutate({ capability: def.capability, provider: p.id })}
          >
            {p.id === selection.provider && <Icon name="check" size="sm" />}
            {p.label}
          </button>
        ))}
      </div>

      {def.providers.map(
        (provider) =>
          provider.tierNote && (
            <p className="set-note" key={`${provider.id}-tier-note`}>
              <b>{provider.label}:</b> {provider.tierNote}
            </p>
          ),
      )}

      {def.capability === 'mesh' && chosen?.id === HUNYUAN3D.id && (
        <div className="prov-region">
          <label htmlFor="hunyuan-region">Tencent processing region</label>
          <select
            id="hunyuan-region"
            aria-label="Tencent Hunyuan3D region"
            value={hunyuanRegion}
            disabled={readOnly || select.isPending}
            onChange={(event) =>
              select.mutate({
                capability: 'mesh',
                provider: HUNYUAN3D.id,
                model: selection.model,
                region: event.currentTarget.value,
              })
            }
          >
            <option value="" disabled>
              Choose a region…
            </option>
            {HUNYUAN_REGIONS.map((region) => (
              <option key={region.id} value={region.id}>
                {region.label}
              </option>
            ))}
          </select>
          <span>
            Required. Tencent only serves these three regions, and every poll stays in the region
            where its job was submitted.
          </span>
        </div>
      )}

      {chosen?.id === HUNYUAN3D.id && !hunyuanRegion && (
        <p className="prov-gap">
          <b>Choose a Tencent processing region.</b> Concept 3D stays off until the project records
          one; Wobu will not guess where to send its images.
        </p>
      )}

      {def.model && chosen && (
        <ModelField
          key={chosen.id}
          capability={def.capability}
          provider={chosen.id}
          model={selection.model}
          readOnly={readOnly}
        />
      )}

      {missing && chosen && (
        <p className="prov-gap">
          <b>{chosen.label} selected — no key on this machine.</b> {def.used} stays off until one is
          added, rather than failing once a job is already running.
          <button
            className="btn-mini"
            onClick={() => onAddKey(chosen.credentials[0]?.id ?? chosen.id)}
          >
            Add key
          </button>
        </p>
      )}

      {/* A project written by a build with more adapters than this one, which
          the shared selection makes possible by design. Saying what it names is
          the only way the user can tell "nothing is chosen" from "something is
          chosen that this Wobu cannot run". */}
      {selection.provider && !chosen && (
        <p className="prov-gap">
          <b>
            This project selects <code>{selection.provider}</code>, which this version of Wobu does
            not have.
          </b>{' '}
          Choosing one above replaces it for everyone.
        </p>
      )}

      {/* What actually runs when nobody has chosen. Left unsaid, the row would
          read as "off" for every world made before this pane existed — and
          those worlds do enhance, at this provider, on this user's key. */}
      {!selection.provider && def.fallback && (
        <p className="set-note">
          Nothing chosen, so {def.used} uses <b>{def.fallback.label}</b> — this build&rsquo;s
          default. Picking one writes it down, and everyone who opens this world gets it.
        </p>
      )}

      {def.activeNote && <p className="set-note">{def.activeNote}</p>}
    </div>
  )
}

const DEFAULT_COMFYUI_ENDPOINT = 'http://127.0.0.1:8188'

function ComfyEndpointSettings() {
  const queryClient = useQueryClient()
  const [saved, setSaved] = useState(DEFAULT_COMFYUI_ENDPOINT)
  const [draft, setDraft] = useState(DEFAULT_COMFYUI_ENDPOINT)
  const [probe, setProbe] = useState<ComfyEndpointProbe | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [checking, setChecking] = useState(false)

  useEffect(() => {
    let disposed = false
    void machineSettings().then(
      (settings) => {
        if (disposed) return
        setSaved(settings.comfyuiEndpoint)
        setDraft(settings.comfyuiEndpoint)
        setLoading(false)
      },
      (reason: unknown) => {
        if (disposed) return
        setError(`Could not read this computer's ComfyUI endpoint: ${errorMessage(reason)}`)
        setLoading(false)
      },
    )
    return () => {
      disposed = true
    }
  }, [])

  async function saveAndCheck() {
    setChecking(true)
    setError(null)
    setProbe(null)
    try {
      const settings = await comfyuiEndpointSet(draft)
      setSaved(settings.comfyuiEndpoint)
      setDraft(settings.comfyuiEndpoint)
      void queryClient.invalidateQueries({ queryKey: ['status_bar_backend'] })
      void queryClient.invalidateQueries({ queryKey: ['image_reference_report'] })
      void queryClient.invalidateQueries({ queryKey: ['image_generation_capabilities'] })
      setProbe(await comfyuiEndpointProbe(settings.comfyuiEndpoint))
    } catch (reason) {
      setError(errorMessage(reason))
    } finally {
      setChecking(false)
    }
  }

  return (
    <div className="prov-endpoint" aria-label="ComfyUI connection">
      <div className="prov-key-head">
        <span className="prov-key-name">ComfyUI endpoint</span>
        <span className="badge">local</span>
      </div>
      <label htmlFor="comfyui-endpoint">Server URL</label>
      <div className="prov-endpoint-entry">
        <input
          id="comfyui-endpoint"
          type="url"
          value={draft}
          placeholder={DEFAULT_COMFYUI_ENDPOINT}
          disabled={loading || checking}
          spellCheck={false}
          autoComplete="off"
          aria-describedby="comfyui-endpoint-boundary"
          onChange={(event) => {
            setDraft(event.currentTarget.value)
            setProbe(null)
            setError(null)
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter') void saveAndCheck()
            if (event.key === 'Escape') {
              setDraft(saved)
              setProbe(null)
              setError(null)
            }
          }}
        />
        <button
          className="btn-mini"
          disabled={loading || checking}
          onClick={() => void saveAndCheck()}
        >
          {checking ? 'Checking…' : 'Save and check'}
        </button>
        {draft !== DEFAULT_COMFYUI_ENDPOINT && (
          <button
            className="btn-mini"
            disabled={checking}
            onClick={() => {
              setDraft(DEFAULT_COMFYUI_ENDPOINT)
              setProbe(null)
              setError(null)
            }}
          >
            Use default
          </button>
        )}
      </div>
      <p className="set-note" id="comfyui-endpoint-boundary">
        Saved in Wobu&rsquo;s application data on this computer, never in <code>project.json</code>{' '}
        or the project folder. Image generation, scene composition, replay and local mesh requests
        all use this route; the current Concept 3D UI can only view and export existing GLBs, so it
        cannot start that local mesh request yet. A non-loopback server receives the prompts and
        reference images those jobs need, so enter only a server you trust. URL credentials are
        rejected; an authenticating proxy must be configured outside Wobu.
      </p>
      {probe && (
        <p className={probe.ok ? 'prov-probe is-ok' : 'prov-probe is-bad'} role="status">
          <Icon name={probe.ok ? 'check' : 'x'} size="sm" />
          {probe.message}
        </p>
      )}
      {error && (
        <p className="prov-probe is-bad" role="alert">
          <Icon name="x" size="sm" />
          {error}
        </p>
      )}
    </div>
  )
}

/**
 * The model id, as free text.
 *
 * Not a dropdown, and that is the point: model ids move faster than anything
 * else in `docs/08-providers.md`, nothing validates this against a list, and a
 * model released next month has to work without a release of ours. Empty means
 * the adapter's own default, which is the answer that stays right on its own.
 */
function ModelField({
  capability,
  provider,
  model,
  readOnly,
}: {
  capability: Capability
  provider: string
  model: string | undefined
  readOnly: boolean
}) {
  const select = useSelectProvider()
  const [draft, setDraft] = useState(model ?? '')

  function commit() {
    const next = draft.trim()
    if (next === (model ?? '')) return
    select.mutate({ capability, provider, model: next || undefined })
  }

  return (
    <div className="prov-model">
      <span className="set-label">Model</span>
      <input
        type="text"
        value={draft}
        placeholder="the provider's default"
        disabled={readOnly}
        spellCheck={false}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') e.currentTarget.blur()
          if (e.key === 'Escape') setDraft(model ?? '')
        }}
      />
    </div>
  )
}

/** How a key's provenance reads, and the colour it reads in. */
const SOURCE_LABEL = {
  keychain: 'in the keychain',
  environment: 'from the environment',
} as const

function ProviderKeys({
  provider,
  statuses,
  model,
  keychainDown,
  focus,
  onFocused,
}: {
  provider: ProviderDef
  statuses: KeyStatus[]
  /** The model this project selected, so a check tests what Enhance would run. */
  model: string | undefined
  keychainDown: boolean
  focus: string | null
  onFocused: () => void
}) {
  const [probe, setProbe] = useState<ProbeResult | null>(null)
  const [checking, setChecking] = useState(false)

  async function check() {
    setChecking(true)
    try {
      setProbe(await providerProbe(provider.id, model))
    } catch (e) {
      report(e, `Could not check the ${provider.label} key`)
    } finally {
      setChecking(false)
    }
  }

  const mine = provider.credentials.map((c) => statuses.find((s) => s.provider === c.id))
  const ready = mine.every((s) => s?.source)
  // Once per provider rather than once per credential: Tencent's pair would
  // otherwise print the same paragraph twice under the same heading.
  const fromEnvironment = mine.some((s) => s?.source === 'environment')

  return (
    <div className="prov-key">
      <div className="prov-key-head">
        <span className="prov-key-name">{provider.label}</span>
        {provider.where && (
          <span className="prov-key-where">a key comes from {provider.where}</span>
        )}
      </div>

      {provider.instead && <p className="set-note">{provider.instead}</p>}

      {provider.hunyuanOnboarding && <HunyuanOnboarding />}

      {provider.credentials.map((credential) => (
        <CredentialRow
          key={credential.id}
          credential={credential}
          multiple={provider.credentials.length > 1}
          status={statuses.find((s) => s.provider === credential.id)}
          keychainDown={keychainDown}
          focus={focus}
          onFocused={onFocused}
          onChanged={() => setProbe(null)}
        />
      ))}

      {fromEnvironment && (
        <p className="set-note">
          Read from a <code>.env</code> at the repository root — a development fallback that only
          exists in this build. There is nothing in the keychain behind it, so there is nothing here
          to remove: to change it, edit that file. A key saved above takes priority over it.
        </p>
      )}

      {provider.checkable && ready && (
        <div className="set-acts">
          <button className="btn-mini" onClick={() => void check()} disabled={checking}>
            <Icon name="check" size="sm" />
            {checking ? 'Checking…' : 'Check this key'}
          </button>
          <span className="prov-cost">
            asks for one description and stops it after a couple of dozen tokens — a fraction of a
            penny, and nothing at all if the key is refused
          </span>
        </div>
      )}

      {probe && (
        <p className={probe.ok ? 'prov-probe is-ok' : 'prov-probe is-bad'}>
          <Icon name={probe.ok ? 'check' : 'x'} size="sm" />
          {probe.message}{' '}
          <span className="prov-probe-cost">
            {probe.usage.inputTokens + probe.usage.cachedInputTokens} in /{' '}
            {probe.usage.outputTokens} out tokens.
          </span>
        </p>
      )}
    </div>
  )
}

/** The account-side work required before a Tencent key can safely be pasted. */
function HunyuanOnboarding() {
  async function visit(url: string, what: string) {
    try {
      await openUrl(url)
    } catch (error) {
      report(error, `Could not open ${what}`)
    }
  }

  return (
    <div className="prov-onboarding" aria-label="Tencent Hunyuan3D setup">
      <p className="prov-alert">
        <b>Do not paste a root-account key.</b> A root SecretKey controls the whole Tencent Cloud
        account. Create a dedicated CAM sub-account and paste only that sub-account&rsquo;s key
        here.
      </p>
      <ol>
        <li>
          <button
            className="btn-mini"
            onClick={() => void visit('https://console.tencentcloud.com/hunyuan', 'Hunyuan 3D')}
          >
            Activate Hunyuan 3D
          </button>
          <span>Sign in as the account owner, accept the service terms and activate it first.</span>
        </li>
        <li>
          <button
            className="btn-mini"
            onClick={() => void visit('https://console.tencentcloud.com/cam', 'CAM users')}
          >
            Open CAM users
          </button>
          <span>
            Create a dedicated sub-account and attach Tencent&rsquo;s verified{' '}
            <code>QcloudAI3DFullAccess</code> managed policy. Tencent does not currently publish
            this policy&rsquo;s action JSON, so Wobu does not suggest an unverified custom prefix.
          </span>
        </li>
        <li>
          <button
            className="btn-mini"
            onClick={() => void visit('https://console.tencentcloud.com/cam/capi', 'CAM API keys')}
          >
            Create sub-account API key
          </button>
          <span>
            Create the SecretId/SecretKey while using that sub-account. Tencent shows SecretKey
            once; store it immediately, then paste the pair below.
          </span>
        </li>
      </ol>
    </div>
  )
}

/**
 * One key field.
 *
 * The pasted key never becomes React state, and that is deliberate rather than
 * stylistic. It is read off the DOM node at the moment Save is pressed, handed
 * to the command, and the node is blanked — so no render captures it, no
 * mutation cache holds it, and there is no component state for a future
 * devtools panel or error boundary to serialise. The Rust side already
 * guarantees a key never comes *back* across the bridge (`keys.rs`); that
 * guarantee is worth little if the webview keeps a copy of the one it sent.
 */
function CredentialRow({
  credential,
  multiple,
  status,
  keychainDown,
  focus,
  onFocused,
  onChanged,
}: {
  credential: Credential
  /** Whether the label has to be shown — a lone "API key" row does not need it. */
  multiple: boolean
  status: KeyStatus | undefined
  keychainDown: boolean
  focus: string | null
  onFocused: () => void
  onChanged: () => void
}) {
  const field = useRef<HTMLInputElement>(null)
  const { save, saving } = useSetProviderKey()
  const remove = useDeleteProviderKey()
  const [editing, setEditing] = useState(false)

  const source = status?.source ?? null
  const wanted = focus === credential.id

  useEffect(() => {
    if (!wanted) return
    setEditing(true)
    onFocused()
  }, [wanted, onFocused])

  // Deliberately after the state above rather than inside the click handler: the
  // field does not exist until `editing` is true, so focusing it has to wait for
  // the render that creates it.
  useEffect(() => {
    if (editing) field.current?.focus()
  }, [editing])

  async function submit() {
    const input = field.current
    if (!input) return
    const key = input.value
    if (!key.trim()) return
    // Blanked before the await, not after: a save that fails still must not
    // leave the key sitting in the DOM, and the value is already captured.
    input.value = ''
    try {
      await save(credential.id, key)
      close()
      onChanged()
      toast('Key saved to this computer’s keychain.')
    } catch (e) {
      report(e, 'Could not save that key')
    }
  }

  /**
   * Leave the field, taking whatever is in it.
   *
   * Blanked rather than merely unmounted. React drops the node either way and
   * the string becomes collectable, but "becomes collectable" is a claim about
   * an engine and this is a claim about the code — and a half-typed key
   * abandoned on Escape is the case nobody would think to check.
   */
  function close() {
    if (field.current) field.current.value = ''
    setEditing(false)
  }

  async function discard() {
    try {
      const removal = await remove.mutateAsync(credential.id)
      onChanged()
      toast(
        removal.status.source
          ? 'Removed from the keychain — this provider is still configured from the environment.'
          : 'Key removed from this computer.',
      )
    } catch (e) {
      report(e, 'Could not remove that key')
    }
  }

  return (
    <div className="prov-cred">
      <span className="prov-cred-label">{multiple ? credential.label : 'Key'}</span>

      {editing ? (
        <div className="prov-cred-entry">
          <input
            ref={field}
            type="password"
            placeholder={`Paste the ${credential.label.toLowerCase()}`}
            // Nothing offers to remember it, nothing sends it to a spell
            // checker, and nothing suggests it back into another field.
            autoComplete="off"
            autoCorrect="off"
            spellCheck={false}
            disabled={keychainDown}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void submit()
              if (e.key === 'Escape') close()
            }}
          />
          <button
            className="btn-mini"
            onClick={() => void submit()}
            disabled={keychainDown || saving === credential.id}
          >
            {saving === credential.id ? 'Saving…' : 'Save'}
          </button>
          <button className="btn-mini" onClick={close}>
            Cancel
          </button>
        </div>
      ) : (
        <>
          <span className={source ? 'prov-cred-state' : 'prov-cred-state is-absent'}>
            <span
              className={source === 'keychain' ? 'dot dot-ok' : source ? 'dot dot-warn' : 'dot'}
            />
            {source ? SOURCE_LABEL[source] : 'no key on this machine'}
          </span>
          <div className="set-acts prov-cred-acts">
            <button className="btn-mini" onClick={() => setEditing(true)} disabled={keychainDown}>
              {source === 'keychain' ? 'Replace' : 'Add key'}
            </button>
            {/* Only for a key this pane can actually remove. An environment
                key has no keychain entry behind it, so a Remove button here
                would report "nothing was removed" and change nothing — which
                reads as the app ignoring the click. */}
            {source === 'keychain' && (
              <button
                className="btn-mini"
                onClick={() => void discard()}
                disabled={remove.isPending}
              >
                <Icon name="trash" size="sm" />
                Remove
              </button>
            )}
          </div>
        </>
      )}
    </div>
  )
}

/**
 * The local SQLite index.
 *
 * Rebuild is offered prominently on purpose. The index is disposable by design
 * — every fact lives in the Markdown — and that property is worth surfacing
 * rather than hiding, because it makes "the navigator is showing something that
 * isn't there" fixable by the user instead of a support conversation.
 */
function Storage() {
  const qc = useQueryClient()
  const [info, setInfo] = useState<IndexInfo | null>(null)
  const [confirming, setConfirming] = useState(false)
  const [busy, setBusy] = useState(false)

  const refresh = useCallback(async () => {
    try {
      setInfo(await indexInfo())
    } catch (e) {
      report(e, 'Could not read the index')
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  async function rebuild() {
    setConfirming(false)
    setBusy(true)
    try {
      await indexRebuild()
      invalidateWorld(qc)
      toast('Index rebuilt from the project folder.')
    } catch (e) {
      report(e, 'Could not rebuild the index')
    } finally {
      setBusy(false)
      void refresh()
    }
  }

  if (!info) return null

  return (
    <section className="set-sec">
      <h3>Storage</h3>
      <p className="set-note">
        The index makes the world searchable. It holds no original copy of anything — every fact is
        in the project folder as Markdown — so deleting or rebuilding it is always safe.
      </p>

      <div className="set-row">
        <span className="set-label">Index</span>
        <code className="set-path">{info.path}</code>
      </div>
      <div className="set-row">
        <span className="set-label">Size</span>
        <span className="set-value">
          {formatSize(info.sizeBytes)} · {info.nodeCount} {info.nodeCount === 1 ? 'node' : 'nodes'}
        </span>
      </div>

      <div className="set-acts">
        <button className="btn-mini" onClick={() => setConfirming(true)} disabled={busy}>
          <Icon name="refresh" size="sm" />
          {busy ? 'Rebuilding…' : 'Rebuild index'}
        </button>
      </div>

      {confirming && (
        <ConfirmSheet
          title="Rebuild the index?"
          body="Every node file is read again from the project folder. Nothing you have written is touched — the index is derived from it. On a large world over a network share this can take a while."
          confirmLabel="Rebuild"
          onConfirm={() => void rebuild()}
          onCancel={() => setConfirming(false)}
        />
      )}
    </section>
  )
}

function EditorPrefs() {
  const delay = useSettings((s) => s.autosaveDelay)
  const setDelay = useSettings((s) => s.setAutosaveDelay)

  return (
    <section className="set-sec">
      <h3>Editor</h3>
      <div className="set-row set-row-col">
        <label className="set-label" htmlFor="settings-autosave-delay">
          Autosave after
        </label>
        <div className="set-slider">
          <input
            id="settings-autosave-delay"
            type="range"
            min={AUTOSAVE_MIN}
            max={AUTOSAVE_MAX}
            step={50}
            value={delay}
            aria-valuetext={`${(delay / 1000).toFixed(2)} seconds`}
            onChange={(e) => setDelay(Number(e.target.value))}
          />
          <span className="set-value">{(delay / 1000).toFixed(2)}s</span>
          {delay !== AUTOSAVE_DEFAULT && (
            <button className="btn-mini" onClick={() => setDelay(AUTOSAVE_DEFAULT)}>
              Reset
            </button>
          )}
        </div>
      </div>
      <p className="set-note">
        How long typing has to stop before the file is written. Shorter means less to lose if
        something goes wrong; longer means fewer writes, which matters on a network share where
        every save is a round trip. It applies to the pane you already have open.
      </p>
    </section>
  )
}

/**
 * There is no theme control, and that is deliberate rather than unfinished.
 *
 * The palette in `styles/tokens.css` is the design — ported verbatim from the
 * prototype and specified in docs/03-ui-layout.md, down to the influence layer
 * colours that carry meaning in the Inspector. A light theme is a second
 * palette that has to be designed, not a switch that has not been wired, and
 * inventing one here would be picking the product's visual identity from a
 * settings pane.
 */
function Appearance() {
  const scale = useSettings((s) => s.uiScale)
  const setScale = useSettings((s) => s.setUiScale)
  const pct = Math.round(scale * 100)

  return (
    <section className="set-sec">
      <h3>Appearance</h3>
      <div className="set-row set-row-col">
        <label className="set-label" htmlFor="settings-interface-scale">
          Interface scale
        </label>
        <div className="set-slider">
          <input
            id="settings-interface-scale"
            type="range"
            min={SCALE_MIN}
            max={SCALE_MAX}
            step={SCALE_STEP}
            value={scale}
            aria-valuetext={`${pct} percent`}
            onChange={(e) => setScale(Number(e.target.value))}
          />
          <span className="set-value">{pct}%</span>
          {scale !== 1 && (
            <button className="btn-mini" onClick={() => setScale(1)}>
              Reset
            </button>
          )}
        </div>
      </div>
      <p className="set-note">
        Scales the whole interface, not just the text — the navigator and inspector are fixed
        widths, so type alone would grow inside boxes that did not.
      </p>
    </section>
  )
}

function About() {
  const [info, setInfo] = useState<AboutInfo | null>(null)

  useEffect(() => {
    void aboutInfo().then(setInfo, (e: unknown) => report(e, 'Could not read the version'))
  }, [])

  if (!info) return null

  return (
    <section className="set-sec">
      <h3>About</h3>
      <div className="set-row">
        <span className="set-label">Version</span>
        <span className="set-value">wobu {info.appVersion}</span>
      </div>
      <div className="set-row">
        <span className="set-label">Project</span>
        <span className="set-value">schema v{info.projectSchemaVersion}</span>
      </div>
      <div className="set-row">
        <span className="set-label">Index</span>
        <span className="set-value">schema v{info.indexSchemaVersion}</span>
      </div>
      <p className="set-note">
        The project schema is the on-disk format of the folder — a world written by a newer Wobu
        than this one cannot be opened here, rather than opened and quietly stripped of what this
        build does not understand. The index schema is local only; a change to it rebuilds on next
        open, which is what a long pause after an update is.
      </p>
    </section>
  )
}

const LEVEL_HELP: Record<LogLevel, string> = {
  off: 'Records nothing at all, errors included. The file stops growing and stops being useful.',
  error: 'Only failures.',
  warn: 'Failures, and the things that recovered.',
  info: 'The default. Failures plus which project was opened and when.',
  debug: 'Adds the technical detail behind each failure. Use this to reproduce something once.',
}

/**
 * The log, and the two things a user actually needs to do with it: change how
 * much it records, and get at the file.
 *
 * Reading it back is offered on purpose. The file is redacted on write, but
 * "trust us" is not a reasonable thing to ask of someone about to paste it into
 * a public issue tracker — so they can look first.
 */
function Diagnostics() {
  const [info, setInfo] = useState<LogInfo | null>(null)
  const [preview, setPreview] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const refresh = useCallback(async () => {
    try {
      setInfo(await logInfo())
    } catch (e) {
      report(e, 'Could not read the log settings')
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  // While the contents are on screen they are kept current. A snapshot would be
  // the wrong default for a pane whose whole job is "this is what you are about
  // to send" — the app keeps logging, and a preview that quietly falls behind
  // the file understates what is in it. Only runs while expanded.
  const open = preview !== null
  useEffect(() => {
    if (!open) return
    const id = window.setInterval(() => {
      void logTail(200).then(setPreview, () => {
        /* a transient read failure is not worth a toast every two seconds */
      })
      void refresh()
    }, 2000)
    return () => window.clearInterval(id)
  }, [open, refresh])

  async function changeLevel(level: LogLevel) {
    // Optimistic: the control is a radio group and a lagging selection reads as
    // a dropped click. A failure re-reads the truth below.
    setInfo((prev) => (prev ? { ...prev, level } : prev))
    try {
      await logSetLevel(level)
    } catch (e) {
      report(e, 'Could not change the log level')
    } finally {
      void refresh()
    }
  }

  async function reveal() {
    try {
      await logReveal()
    } catch (e) {
      report(e, 'Could not show the log')
    }
  }

  async function togglePreview() {
    if (preview !== null) {
      setPreview(null)
      return
    }
    setBusy(true)
    try {
      setPreview(await logTail(200))
    } catch (e) {
      report(e, 'Could not read the log')
    } finally {
      setBusy(false)
    }
  }

  if (!info) return null

  return (
    <section className="set-sec">
      <h3>Diagnostics</h3>
      <p className="set-note">
        Wobu sends nothing anywhere. When something goes wrong, this file is the only account of it
        — it stays on this machine until you hand it over yourself.
      </p>

      <div className="set-row">
        <span className="set-label">Log file</span>
        <code className="set-path">{info.path}</code>
      </div>
      <div className="set-row">
        <span className="set-label">Size</span>
        <span className="set-value">
          {info.exists ? formatSize(info.sizeBytes) : 'nothing recorded yet'}
        </span>
      </div>

      <div className="set-row set-row-col">
        <span className="set-label">Level</span>
        <div className="set-levels">
          {LOG_LEVELS.map((l) => (
            <button
              key={l}
              className={l === info.level ? 'btn-mini is-on' : 'btn-mini'}
              aria-pressed={l === info.level}
              onClick={() => void changeLevel(l)}
            >
              {l}
            </button>
          ))}
        </div>
        <p className="set-note">{LEVEL_HELP[info.level]}</p>
      </div>

      <div className="set-acts">
        <button className="btn-mini" onClick={() => void reveal()}>
          <Icon name="folder" size="sm" />
          Reveal log file
        </button>
        <button className="btn-mini" onClick={() => void togglePreview()} disabled={busy}>
          <Icon name="search" size="sm" />
          {preview !== null ? 'Hide contents' : busy ? 'Reading…' : 'Show contents'}
        </button>
        {preview !== null && (
          <button
            className="btn-mini"
            onClick={() => {
              void navigator.clipboard.writeText(preview).then(
                () => toast('Log copied.'),
                (e: unknown) => report(e, 'Could not copy the log'),
              )
            }}
          >
            <Icon name="copy" size="sm" />
            Copy
          </button>
        )}
      </div>

      {preview !== null && (
        <pre className="set-log">{preview.trim() ? preview : 'Nothing has been recorded yet.'}</pre>
      )}
    </section>
  )
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
