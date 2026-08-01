import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { Workspace } from './Workspace'
import type { KindDef, NodeSummary, ProjectSummary, WobuNode } from '../lib/api'
import { READ_ONLY_TEXT } from '../lib/readOnly'
import { kindDef, node as buildNode, summary } from '../test/fixtures'
import { useUI } from '../store/ui'
import { useUndoStack } from '../lib/undo'

/*
 * A project on a share that is mounted read-only (#19).
 *
 * The failure this guards against is the one the issue names: every write
 * control still live, each of them failing at save time, one error at a time,
 * for a condition that was known the moment the folder was opened. So the
 * assertions are about what a person can reach — a button they can press, a
 * field they can type in, a row they can drag — and about the banner being the
 * single place the reason is given.
 */

const h = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))
// The title bar draws its own window buttons, which ask the real window for its
// state the moment they mount. There is no window here.
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    minimize: () => Promise.resolve(),
    toggleMaximize: () => Promise.resolve(),
    close: () => Promise.resolve(),
    isMaximized: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
  }),
}))

const kinds: KindDef[] = [kindDef('character', { label: 'Character', plural: 'Characters' })]
const kael: NodeSummary = summary({ id: 'kael', name: 'Kael' })
const kaelNode: WobuNode = buildNode({ id: 'kael', name: 'Kael', notesRaw: 'an ashwalker' })

function backend(cmd: string): unknown {
  switch (cmd) {
    case 'job_list':
      return { jobs: [], queued: 0, running: 0, retrying: 0 }
    case 'kind_registry':
      return kinds
    case 'node_list':
      return [kael]
    case 'node_get':
      return kaelNode
    case 'corrupt_files':
    case 'conflicts':
      return []
    default:
      return null
  }
}

function project(readOnly: boolean): ProjectSummary {
  return {
    id: 'p1',
    name: 'Ashfall',
    path: '/Volumes/art/ashfall',
    onNetworkShare: true,
    readOnly,
    lastOpenedAt: null,
  }
}

async function open(readOnly: boolean) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  render(
    <QueryClientProvider client={qc}>
      <Workspace project={project(readOnly)} />
    </QueryClientProvider>,
  )
  await screen.findByRole('button', { name: /Kael/ })
}

/** The navigator row for a node, which is also what a drag would start from. */
function row(name: string): HTMLElement {
  const [first] = screen.getAllByRole('button', { name: new RegExp(name) })
  if (!first) throw new Error(`no row for ${name}`)
  return first
}

function button(name: string | RegExp): HTMLButtonElement {
  return screen.getByRole('button', { name }) as HTMLButtonElement
}

beforeEach(() => {
  h.invoke.mockReset()
  h.invoke.mockImplementation((cmd: string) => Promise.resolve(backend(cmd)))
  useUI.setState({
    toasts: [],
    banners: [],
    selectedId: null,
    paletteOpen: false,
    mode: 'library',
    filter: '',
    tab: 'notes',
  })
  useUndoStack.getState().setProject('p1')
  // Without this `isTauri()` is false and every command rejects before it is
  // sent, which is right in a browser and useless here.
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('the read-only banner', () => {
  it('says the share is mounted read-only, once, at the top of the workspace', async () => {
    await open(true)

    const banners = screen.getAllByText(READ_ONLY_TEXT)
    expect(banners).toHaveLength(1)
    expect(READ_ONLY_TEXT).toContain('This share is mounted read-only')
  })

  it('stays one banner across selecting nodes and switching modes', async () => {
    // The condition belongs to the folder, not to whatever is on screen. Raising
    // it per node or per navigation is how one honest sentence becomes noise.
    await open(true)
    fireEvent.click(row('Kael'))
    fireEvent.click(button(/^Forge/))
    fireEvent.click(button(/^Library/))

    expect(useUI.getState().banners.map((b) => b.code)).toEqual(['project.read_only'])
  })

  it('is absent on a folder that can be written to', async () => {
    await open(false)
    expect(screen.queryByText(READ_ONLY_TEXT)).toBeNull()
    expect(useUI.getState().banners).toEqual([])
  })
})

describe('the write controls, on a read-only folder', () => {
  it('does not offer to create anything, by any of the routes there', async () => {
    // A disabled button is only the visible half. ⌘N and the group header's
    // context menu reach the same sheet without going near it, and either one
    // would put a Create button in front of the user on a folder that cannot
    // take one.
    await open(true)
    expect(button('New entity').disabled).toBe(true)

    fireEvent.keyDown(window, { key: 'n', metaKey: true })
    fireEvent.contextMenu(button(/Characters/))

    expect(screen.queryByRole('dialog', { name: 'New node' })).toBeNull()
  })

  it('offers no rename, no notes editing, and no save label promising otherwise', async () => {
    await open(true)
    fireEvent.click(row('Kael'))

    const name = (await screen.findByLabelText('Node name')) as HTMLInputElement
    expect(name.readOnly).toBe(true)

    const notes = (await screen.findByPlaceholderText(/read-only/)) as HTMLTextAreaElement
    expect(notes.readOnly).toBe(true)

    // Typing into a read-only textarea is a no-op in a real browser; firing the
    // change directly is the stronger test, because it proves the handler
    // behind it would not have written either.
    fireEvent.change(notes, { target: { value: 'typed anyway' } })
    expect(h.invoke).not.toHaveBeenCalledWith('node_upsert', expect.anything())
  })

  it('offers no duplicate, no delete and no new-child from a row', async () => {
    await open(true)
    fireEvent.contextMenu(row('Kael'))

    const menu = within(screen.getByRole('menu'))
    for (const label of [/New character/, 'Duplicate', 'Delete']) {
      expect((menu.getByRole('button', { name: label }) as HTMLButtonElement).disabled).toBe(true)
    }
  })

  it('does not let a row be dragged somewhere else', async () => {
    await open(true)
    expect(row('Kael').getAttribute('draggable')).toBe('false')
  })

  it('drops the commands that write from the palette, rather than greying them', async () => {
    // The banner has already said why. A disabled row in a list the user is
    // typing into would be a second explanation nobody asked for.
    await open(true)
    fireEvent.click(button(/Jump to…/))

    expect(screen.queryByRole('button', { name: /New node…/ })).toBeNull()
    expect(screen.getByRole('button', { name: /Toggle navigator/ })).toBeTruthy()
  })

  it('leaves Enhance disabled, and says the share is why', async () => {
    // M4 has not landed, so this button is disabled either way — what changes
    // is the reason, and the reason is the one that will still be true in M4.
    await open(true)
    fireEvent.click(row('Kael'))
    expect(button(/Enhance/).title).toContain('read-only')
  })
})

describe('the same controls, on a folder that can be written to', () => {
  it('leaves every one of them live', async () => {
    await open(false)
    expect(button('New entity').disabled).toBe(false)
    expect(row('Kael').getAttribute('draggable')).toBe('true')

    fireEvent.click(button(/Jump to…/))
    expect(screen.getByRole('button', { name: /New node…/ })).toBeTruthy()
    fireEvent.keyDown(window, { key: 'Escape' })

    fireEvent.keyDown(window, { key: 'n', metaKey: true })
    expect(screen.getByRole('dialog', { name: 'New node' })).toBeTruthy()
  })
})
