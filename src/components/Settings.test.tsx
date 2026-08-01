import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { Settings } from './Settings'
import type { KeyStatus, ProviderSelections } from '../lib/api'
import { useUI } from '../store/ui'

/*
 * The providers pane, which is the one surface in the app where a credential
 * passes through the renderer and where two facts that look alike — "this
 * project selects Gemini" and "this machine has a Gemini key" — have to stay
 * visibly apart. Both failures are quiet: a key retained somewhere in the
 * webview is invisible until a crash report carries it out, and a collaborator
 * who mistakes a shared selection for a shared key finds out at generate time.
 * So the assertions below are about what is *not* held and what is actually on
 * screen, not about markup.
 */

const h = vi.hoisted(() => ({ invoke: vi.fn(), openUrl: vi.fn(), reveal: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: h.openUrl, revealItemInDir: h.reveal }))

/** What the backend answers, per command. Overwritten per test. */
let keyStatuses: KeyStatus[] = []
let selections: ProviderSelections = { providers: {}, readOnly: false }

function unconfigured(provider: string): KeyStatus {
  return { provider, source: null, keychain: 'ready' }
}

/** Every command the Settings surface calls, so no section falls over. */
function backend(cmd: string, args: Record<string, unknown> | undefined): unknown {
  switch (cmd) {
    case 'provider_key_status':
      return keyStatuses
    case 'project_providers':
      return selections
    case 'provider_key_set':
      return { provider: args?.provider, source: 'keychain', keychain: 'ready' }
    case 'provider_key_delete':
      return { removed: true, status: unconfigured(String(args?.provider)) }
    case 'project_provider_select':
      return selections
    case 'provider_probe':
      return {
        provider: args?.provider,
        model: 'claude-sonnet-5',
        ok: true,
        message: 'Anthropic took the key and started writing.',
        code: null,
        usage: { inputTokens: 400, cachedInputTokens: 0, outputTokens: 24 },
      }
    // The other sections. Answered rather than stubbed out so the pane under
    // test renders inside the real page.
    case 'index_info':
      return { path: '/tmp/index.sqlite', sizeBytes: 1024, nodeCount: 3 }
    case 'log_info':
      return { path: '/tmp/wobu.log', level: 'info', exists: true, sizeBytes: 12 }
    case 'about_info':
      return { appVersion: '0.1.0', projectSchemaVersion: 1, indexSchemaVersion: 1 }
    default:
      return null
  }
}

let qc: QueryClient

async function open() {
  qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  render(
    <QueryClientProvider client={qc}>
      <Settings />
    </QueryClientProvider>,
  )
  await screen.findByText('What this project uses')
}

/**
 * The first add-key affordance on screen. When a capability is missing a key
 * that is the one inside the shared band, which is the one being tested.
 */
function addKey(): HTMLElement {
  const [first] = screen.getAllByRole('button', { name: 'Add key' })
  if (!first) throw new Error('nothing on screen offers to add a key')
  return first
}

/** Everything React Query is holding, as text a key could be found in. */
function cached(): string {
  const queries = qc
    .getQueryCache()
    .getAll()
    .map((q) => ({ key: q.queryKey, state: q.state }))
  const mutations = qc
    .getMutationCache()
    .getAll()
    .map((m) => m.state)
  return JSON.stringify({ queries, mutations })
}

beforeEach(() => {
  h.invoke.mockReset()
  h.openUrl.mockReset()
  h.reveal.mockReset()
  h.openUrl.mockResolvedValue(undefined)
  h.invoke.mockImplementation((cmd: string, args?: Record<string, unknown>) =>
    Promise.resolve(backend(cmd, args)),
  )
  keyStatuses = [
    unconfigured('anthropic'),
    unconfigured('gemini'),
    unconfigured('tencent-secret-id'),
    unconfigured('tencent-secret-key'),
  ]
  selections = { providers: {}, readOnly: false }
  useUI.setState({ toasts: [], banners: [] })
  // Without this `isTauri()` is false and every command rejects before it is
  // sent, which is right in a browser and useless here.
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('the key a user pastes', () => {
  it('goes to the backend and is kept nowhere the renderer can read it afterwards', async () => {
    // The regression #31 named. Key material crosses this bridge inwards only,
    // and the Rust side guarantees nothing comes back — which is worth very
    // little if the webview keeps its own copy. React Query's mutation cache is
    // the specific trap: it holds `variables` on a settled mutation until
    // something resets it, and it hangs off the QueryClient for the life of the
    // app. So the key is never allowed into a hook, a mutation or a state
    // setter, and this asserts on the caches rather than on the call.
    const KEY = 'sk-ant-api03-pasted-by-the-user'
    await open()

    fireEvent.click(addKey())
    const field = await screen.findByPlaceholderText('Paste the api key')
    fireEvent.change(field, { target: { value: KEY } })
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('provider_key_set', {
        provider: 'anthropic',
        key: KEY,
      }),
    )

    expect(cached()).not.toContain(KEY)
    expect(cached()).not.toContain('pasted-by-the-user')
    expect(document.body.innerHTML).not.toContain(KEY)
    // And the field it was typed into no longer holds it either — a DOM node
    // parked behind a collapsed section is as reachable as any other.
    expect((field as HTMLInputElement).value).toBe('')
  })

  it('is masked and offered to nothing that would remember it', async () => {
    // A settings field that a password manager or a spell checker can read is
    // a key that has left this process by a route nobody audited.
    await open()
    fireEvent.click(addKey())

    const field = (await screen.findByPlaceholderText('Paste the api key')) as HTMLInputElement
    expect(field.type).toBe('password')
    expect(field.getAttribute('autocomplete')).toBe('off')
    expect(field.getAttribute('spellcheck')).toBe('false')
  })
})

describe('a project whose provider has no key here', () => {
  it('says so beside the selection and offers the field, rather than failing at generate time', async () => {
    // The state every collaborator is in on day one: the selection arrived with
    // the folder, the key did not. Without this the first sign of trouble is a
    // queued Enhance failing, which reads as the app being broken.
    selections = { providers: { text: { provider: 'gemini' } }, readOnly: false }
    await open()

    expect(await screen.findByText(/Gemini selected — no key on this machine/)).toBeTruthy()
    expect(screen.getByText(/Enhance stays off until one is added/)).toBeTruthy()

    // And the way out is one click from the sentence, not a hunt through the
    // pane: it opens the field for the credential that is missing.
    fireEvent.click(addKey())
    await waitFor(() => expect(screen.getByPlaceholderText('Paste the api key')).toBeTruthy())
  })

  it('does not say so when the key is there', async () => {
    selections = { providers: { text: { provider: 'gemini' } }, readOnly: false }
    keyStatuses = keyStatuses.map((s) =>
      s.provider === 'gemini' ? { ...s, source: 'keychain' } : s,
    )
    await open()

    expect(screen.queryByText(/no key on this machine\./)).toBeNull()
  })
})

describe('where a key came from', () => {
  it('reads differently for the environment than for the keychain, and offers no Remove', async () => {
    // A developer who sees "configured" without being told it came from a
    // `.env` deletes the key and finds nothing changed. The keychain entry and
    // the development fallback are two different things and only one of them is
    // this pane's to remove.
    keyStatuses = [
      { provider: 'anthropic', source: 'environment', keychain: 'ready' },
      { provider: 'gemini', source: 'keychain', keychain: 'ready' },
      unconfigured('tencent-secret-id'),
      unconfigured('tencent-secret-key'),
    ]
    await open()

    expect(screen.getByText('from the environment')).toBeTruthy()
    expect(screen.getByText('in the keychain')).toBeTruthy()
    expect(screen.getByText(/There is nothing in the keychain behind it/)).toBeTruthy()
    // One Remove, for the one key that is actually stored here.
    expect(screen.getAllByRole('button', { name: /Remove/ })).toHaveLength(1)
  })
})

describe('a machine with no credential store', () => {
  it('explains itself beside the disabled fields instead of raising anything', async () => {
    // A locked login keyring or a headless session is an ordinary machine, not
    // a failure. A toast would be dismissible and would leave the user pressing
    // a Save that silently cannot work.
    keyStatuses = keyStatuses.map((s) => ({ ...s, keychain: 'unavailable' as const }))
    await open()

    expect(screen.getByText(/the login keyring is locked/)).toBeTruthy()
    for (const button of screen.getAllByRole('button', { name: 'Add key' })) {
      expect((button as HTMLButtonElement).disabled).toBe(true)
    }
    expect(useUI.getState().toasts).toHaveLength(0)
    expect(useUI.getState().banners).toHaveLength(0)
  })
})

describe('the shared selection', () => {
  it('requires an explicit supported Tencent processing region and stores it with the project', async () => {
    selections = { providers: { mesh: { provider: 'hunyuan3d' } }, readOnly: false }
    await open()

    const mesh = within(screen.getByRole('group', { name: 'Mesh — Concept 3D' }))
    const region = mesh.getByLabelText('Tencent Hunyuan3D region') as HTMLSelectElement
    expect(region.value).toBe('')
    expect(mesh.getByText(/Concept 3D stays off until the project records one/)).toBeTruthy()
    expect([...region.options].map((option) => option.value)).toEqual([
      '',
      'ap-singapore',
      'na-siliconvalley',
      'eu-frankfurt',
    ])

    fireEvent.change(region, { target: { value: 'eu-frankfurt' } })
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('project_provider_select', {
        capability: 'mesh',
        provider: 'hunyuan3d',
        model: undefined,
        region: 'eu-frankfurt',
      }),
    )
  })

  it('presents local Hunyuan3D as an explicit older tier before it is selected', async () => {
    await open()

    const mesh = within(screen.getByRole('group', { name: 'Mesh — Concept 3D' }))
    expect(mesh.getByRole('button', { name: 'Local 2.1 (ComfyUI)' })).toBeTruthy()
    expect(mesh.getByText(/never an automatic fallback/)).toBeTruthy()
    expect(mesh.getByText(/older, lower-quality Hunyuan3D 2.1/)).toBeTruthy()
    expect(mesh.getByText(/one front image, geometry-only output/)).toBeTruthy()
    expect(mesh.getByText(/at least 10 GB VRAM/)).toBeTruthy()
    expect(mesh.getByText(/no per-job fee/)).toBeTruthy()
    expect(mesh.getByText(/staying on your ComfyUI machine/)).toBeTruthy()
    expect(mesh.getByText(/licence excludes the EU, UK and South Korea/)).toBeTruthy()
  })

  it('records local meshing only when the user explicitly chooses it', async () => {
    await open()

    const mesh = within(screen.getByRole('group', { name: 'Mesh — Concept 3D' }))
    const local = mesh.getByRole('button', { name: 'Local 2.1 (ComfyUI)' })
    expect(local.className).not.toContain('is-on')
    expect(mesh.queryByText(/Nothing chosen, so Concept 3D uses/)).toBeNull()
    fireEvent.click(local)
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('project_provider_select', {
        capability: 'mesh',
        provider: 'comfyui',
        model: undefined,
      }),
    )
  })

  it('does not carry one vendor’s model id across to another', async () => {
    // `claude-sonnet-5` handed to Gemini is a request that fails for a reason
    // nothing on screen explains, and the user would have to know that a model
    // id belongs to exactly one vendor to diagnose it.
    selections = {
      providers: { text: { provider: 'anthropic', model: 'claude-sonnet-5' } },
      readOnly: false,
    }
    await open()

    const text = within(screen.getByRole('group', { name: 'Text — Enhance' }))
    fireEvent.click(text.getByRole('button', { name: 'Gemini' }))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('project_provider_select', {
        capability: 'text',
        provider: 'gemini',
        model: undefined,
      }),
    )
  })

  it('names what runs when the project has chosen nothing', async () => {
    // Every world made before this pane existed selects nothing, and Enhance
    // does not refuse for it — `enhance.rs` falls back to Anthropic. A row that
    // just showed nothing highlighted would describe a project as unconfigured
    // while it was quietly spending at a provider it never named.
    selections = { providers: {}, readOnly: false }
    await open()

    const text = within(screen.getByRole('group', { name: 'Text — Enhance' }))
    expect(text.getByText(/Nothing chosen, so Enhance uses/)).toBeTruthy()
    expect(text.getByText('Anthropic', { selector: 'b' })).toBeTruthy()
  })

  it('is the only thing a read-only project disables — keys are still this machine’s', async () => {
    // Keys are per installation. Locking the whole pane because somebody else's
    // folder is read-only would stop the user configuring their own computer.
    selections = { providers: { text: { provider: 'anthropic' } }, readOnly: true }
    await open()

    expect(screen.getByText(/This folder is read-only/)).toBeTruthy()
    const text = within(screen.getByRole('group', { name: 'Text — Enhance' }))
    expect((text.getByRole('button', { name: 'Gemini' }) as HTMLButtonElement).disabled).toBe(true)
    for (const button of screen.getAllByRole('button', { name: 'Add key' })) {
      expect((button as HTMLButtonElement).disabled).toBe(false)
    }
  })
})

describe('Tencent Hunyuan3D onboarding', () => {
  it('puts activation and a scoped CAM sub-account before the credential fields', async () => {
    await open()

    const setup = within(screen.getByLabelText('Tencent Hunyuan3D setup'))
    expect(setup.getByText(/Do not paste a root-account key/)).toBeTruthy()
    expect(setup.getByText(/dedicated CAM sub-account/)).toBeTruthy()
    expect(setup.getByText(/QcloudAI3DFullAccess/)).toBeTruthy()
    expect(setup.getByText(/does not currently publish this policy’s action JSON/)).toBeTruthy()

    fireEvent.click(setup.getByRole('button', { name: 'Activate Hunyuan 3D' }))
    fireEvent.click(setup.getByRole('button', { name: 'Open CAM users' }))
    fireEvent.click(setup.getByRole('button', { name: 'Create sub-account API key' }))
    await waitFor(() => {
      expect(h.openUrl).toHaveBeenCalledWith('https://console.tencentcloud.com/hunyuan')
      expect(h.openUrl).toHaveBeenCalledWith('https://console.tencentcloud.com/cam')
      expect(h.openUrl).toHaveBeenCalledWith('https://console.tencentcloud.com/cam/capi')
    })
  })
})

describe('checking a key', () => {
  it('says what it will cost before it is pressed, and what it did cost afterwards', async () => {
    // A probe is a paid call on some providers and free on others. Offering one
    // without saying which is how a settings pane spends somebody's money.
    keyStatuses = keyStatuses.map((s) =>
      s.provider === 'anthropic' ? { ...s, source: 'keychain' as const } : s,
    )
    await open()

    expect(screen.getByText(/a fraction of a penny/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /Check this key/ }))

    expect(await screen.findByText(/started writing/)).toBeTruthy()
    expect(screen.getByText(/400 in \/ 24 out tokens\./)).toBeTruthy()
  })

  it('is not offered for a backend that authenticates nothing', async () => {
    // ComfyUI is a server the user runs. Whether it works is a question about a
    // URL answering, not about a credential being valid, and a "check key"
    // button beside it would be asking a question that has no meaning.
    await open()

    expect(screen.getByText(/ComfyUI needs no key/)).toBeTruthy()
    expect(screen.queryAllByRole('button', { name: /Check this key/ })).toHaveLength(0)
  })
})
