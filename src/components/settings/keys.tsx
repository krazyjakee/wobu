import { useEffect, useRef, useState } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'
import type { KeyStatus, ProbeResult } from '../../lib/api'
import { providerProbe } from '../../lib/api'
import { useDeleteProviderKey, useSetProviderKey } from '../../lib/queries'
import { report, toast } from '../../store/ui'
import { Icon } from '../Icon'
import type { Credential, ProviderDef } from './providerDefs'

/** How a key's provenance reads, and the colour it reads in. */
const SOURCE_LABEL = {
  keychain: 'in the keychain',
  environment: 'from the environment',
} as const

export function ProviderKeys({
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
export function HunyuanOnboarding() {
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
export function CredentialRow({
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
  const [editingByChoice, setEditing] = useState(false)

  const source = status?.source ?? null
  const wanted = focus === credential.id
  const editing = editingByChoice || wanted

  // The externally requested row derives its open state from `focus`. The
  // request stays active until this field closes, so no reset render is needed.
  useEffect(() => {
    if (!editing) return
    const input = field.current
    // The shortcut lives in the shared-project band, while the key field is in
    // the machine-local band below it. Focusing normally scrolls in browsers,
    // but WebKit does not consistently move a nested overflow container when a
    // node appeared in the same render. Make the navigation explicit so the
    // click cannot open a field off-screen and look like it did nothing.
    input?.scrollIntoView?.({ block: 'center' })
    input?.focus()
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
    if (wanted) onFocused()
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
