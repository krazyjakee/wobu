import { useCallback, useEffect, useState } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import type { Capability, ComfyEndpointProbe, KeyStatus, ProviderSelection } from '../../lib/api'
import {
  comfyuiEndpointProbe,
  comfyuiEndpointSet,
  errorMessage,
  machineSettings,
} from '../../lib/api'
import { useProviderKeys, useProviderSelections, useSelectProvider } from '../../lib/queries'
import { Icon } from '../Icon'
import { ProviderKeys } from './keys'
import { CAPABILITIES, CREDENTIAL_IDS, HUNYUAN3D, HUNYUAN_REGIONS, KEYED } from './providerDefs'
import type { CapabilityDef, ProviderDef } from './providerDefs'

/**
 * Providers and keys — and above all, which of the two is shared.
 *
 * The pane is two bands rather than one list of providers, and that is the
 * whole design rather than a layout choice. A project's *selection* lives in
 * `project.json` and goes wherever the folder goes: open a world from a shared
 * drive and you are looking at somebody else's decision. A *key* is per
 * installation, sits in this machine's credential storage, and goes nowhere at all. Those
 * two facts are what BYOK means, they are the two facts users get wrong, and a
 * sentence explaining them is worth much less than a layout in which they are
 * obviously separate things — so the pane keeps them apart, labels each band
 * with what it is, and never repeats a key row inside a capability.
 *
 * See `docs/08-providers.md`.
 */
export function Providers() {
  const selections = useProviderSelections()
  const keys = useProviderKeys(CREDENTIAL_IDS)
  const [focus, setFocus] = useState<string | null>(null)
  // Stable identity: it is an effect dependency in every credential row, and a
  // fresh closure per render would re-run all of them on every render.
  const clearFocus = useCallback(() => setFocus(null), [])

  // Key status is an enhancement, not a gate on the action. The native lookup
  // may be waiting on a broken Linux Secret Service, so render usable empty
  // rows immediately and let the query fill in their actual sources.
  const statuses: KeyStatus[] =
    keys.data ?? CREDENTIAL_IDS.map((provider) => ({ provider, source: null, keychain: 'ready' }))
  const selected = selections.data

  // Both are one round trip on mount and neither is worth a spinner; the
  // sections around this one take the same line.
  if (!selected) return null
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
        computer&rsquo;s credential storage, and everyone who opens the same world runs it on their
        own.
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
            This computer&rsquo;s OS credential store is not answering. Add, replace and remove
            still work: Wobu uses its owner-only local credential store until the OS service is
            available.
          </p>
        )}
        {KEYED.map((provider) => (
          <ProviderKeys
            key={provider.id}
            provider={provider}
            statuses={statuses}
            model={chosen.text?.provider === provider.id ? chosen.text?.model : undefined}
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
export function CapabilityRow({
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

export const DEFAULT_COMFYUI_ENDPOINT = 'http://127.0.0.1:8188'

export function ComfyEndpointSettings() {
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
        setError(`Could not read this computer's ComfyUI address: ${errorMessage(reason)}`)
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
        <span className="prov-key-name">ComfyUI address</span>
        <span className="badge">local</span>
      </div>
      <label htmlFor="comfyui-endpoint">Server address</label>
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
        all use this address, including a mesh reconstruction started from an entity&rsquo;s 3D tab.
        Anything other than a server on this computer receives the prompts and reference images
        those jobs need, so enter only a server you trust. A user name and password in the address
        is refused; a server that needs a sign-in has to be put behind something outside Wobu.
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
export function ModelField({
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
