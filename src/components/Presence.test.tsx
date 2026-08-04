import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { Workspace } from './Workspace'
import type { KindDef, NodeSummary, Peer, ProjectSummary, WobuNode } from '../lib/api'
import { PRESENCE_BANNER, STALE_AFTER_SECS } from '../lib/presence'
import { kindDef, node as buildNode, summary } from '../test/fixtures'
import { useUI } from '../store/ui'
import { useUndoStack } from '../lib/undo'

/*
 * Presence, as the four surfaces the issue asks for (#17): the greeting on
 * open, a dot on the navigator row, a passive banner over the editor, and the
 * session count in the status bar.
 *
 * Two failures are worth more than the rest and are what most of this file is
 * about. The first is a session that died still being shown as present — a
 * closed laptop leaves its heartbeat file behind, and a name that never goes
 * away is worse than no name at all. The second is any of this becoming a
 * *reason not to type*: presence is advisory, so every assertion about the
 * banner is paired with one about the controls under it still being live.
 */

const h = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))
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

function peer(over: Partial<Peer> & { sessionId: string }): Peer {
  return { user: 'Nadia', host: 'nas', seenSecsAgo: 4, editing: [], ...over }
}

/** Swapped between polls, the way the folder changes under a real session. */
let present: Peer[] = []

function backend(cmd: string): unknown {
  switch (cmd) {
    case 'kind_registry':
      return kinds
    case 'node_list':
      return [kael]
    case 'node_get':
      return kaelNode
    case 'presence_peers':
      return present
    case 'job_list':
      return { jobs: [], queued: 0, running: 0, retrying: 0 }
    case 'corrupt_files':
    case 'conflicts':
      return []
    default:
      return null
  }
}

const project: ProjectSummary = {
  id: 'p1',
  name: 'Ashfall',
  path: '/Volumes/art/ashfall',
  onNetworkShare: true,
  readOnly: false,
  lastOpenedAt: null,
}

async function open() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  render(
    <QueryClientProvider client={qc}>
      <Workspace project={project} />
    </QueryClientProvider>,
  )
  await screen.findByRole('button', { name: /Kael/ })
  // The greeting waits for the first answer, so nothing here is true until the
  // presence query has actually resolved once.
  await waitFor(() => expect(h.invoke).toHaveBeenCalledWith('presence_peers', undefined))
  await act(async () => {})
  return qc
}

/** Another beat of the heartbeat, with whatever `present` now holds. */
async function poll(qc: QueryClient) {
  await act(async () => {
    await qc.refetchQueries({ queryKey: ['presence_peers'] })
  })
  // React Query hands results to subscribers through its own batched notifier,
  // which `act` does not drain — the render this causes lands a tick later, and
  // without waiting for it every assertion below would read the previous poll.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

function row(name: string): HTMLElement {
  const [first] = screen.getAllByRole('button', { name: new RegExp(name) })
  if (!first) throw new Error(`no row for ${name}`)
  return first
}

const greetings = () => useUI.getState().toasts.map((t) => t.text)
const bannerCodes = () => useUI.getState().banners.map((b) => b.code)

beforeEach(() => {
  present = []
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
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

describe('opening a project somebody else has open', () => {
  it('says so once, in the words the issue asks for', async () => {
    present = [peer({ sessionId: 's1', user: 'Nadia' })]
    const qc = await open()

    expect(greetings()).toEqual(['Nadia has this project open.'])

    // Three more beats, a selection and a mode switch. The greeting belongs to
    // the *open*, and repeating it every ten seconds would be the same sentence
    // six times a minute.
    await poll(qc)
    await poll(qc)
    fireEvent.click(row('Kael'))
    await poll(qc)

    expect(greetings()).toEqual(['Nadia has this project open.'])
  })

  it('says nothing at all when the folder is ours alone', async () => {
    await open()
    expect(greetings()).toEqual([])
  })

  it('does not greet a session that stopped beating', async () => {
    // The heartbeat file outlives the laptop that wrote it. Sixty seconds is the
    // bound `presence.rs` reaps at, and this side holds the same one because it
    // is holding an answer rather than reading the folder.
    present = [peer({ sessionId: 's1', user: 'Nadia', seenSecsAgo: STALE_AFTER_SECS + 5 })]
    await open()

    expect(greetings()).toEqual([])
    expect(screen.queryByText('2 sessions')).toBeNull()
  })
})

describe('a node somebody else has open', () => {
  it('gets a quiet dot on its navigator row', async () => {
    present = [peer({ sessionId: 's1', user: 'Nadia', editing: ['kael'] })]
    await open()

    expect(screen.getByLabelText(/Nadia has this entity open/)).toBeTruthy()
  })

  it('loses the dot when that session goes stale', async () => {
    present = [peer({ sessionId: 's1', user: 'Nadia', editing: ['kael'] })]
    const qc = await open()
    expect(screen.getByLabelText(/Nadia has this entity open/)).toBeTruthy()

    present = [peer({ sessionId: 's1', user: 'Nadia', editing: ['kael'], seenSecsAgo: 600 })]
    await poll(qc)

    expect(screen.queryByLabelText(/Nadia has this entity open/)).toBeNull()
  })

  it('has no dot on a row nobody else is in', async () => {
    present = [peer({ sessionId: 's1', user: 'Nadia', editing: ['someone-else'] })]
    await open()
    expect(screen.queryByLabelText(/has this node open/)).toBeNull()
  })
})

describe('the banner over a node being edited elsewhere', () => {
  it('appears once when that node is opened, and not again on every beat', async () => {
    present = [peer({ sessionId: 's1', user: 'Nadia', editing: ['kael'] })]
    const qc = await open()
    expect(bannerCodes()).toEqual([])

    fireEvent.click(row('Kael'))
    await act(async () => {})
    expect(bannerCodes()).toEqual([PRESENCE_BANNER])
    expect(screen.getAllByText(/Nadia has “Kael” open in another copy of Wobu/)).toHaveLength(1)

    // Each beat brings a fresh answer with a fresh `seenSecsAgo`. The banner is
    // keyed to the situation, not to the poll that reported it.
    for (const seenSecsAgo of [12, 25, 38]) {
      present = [peer({ sessionId: 's1', user: 'Nadia', editing: ['kael'], seenSecsAgo })]
      await poll(qc)
    }

    expect(bannerCodes()).toEqual([PRESENCE_BANNER])
    expect(screen.getAllByText(/Nadia has “Kael” open in another copy of Wobu/)).toHaveLength(1)
  })

  it('stays dismissed once it has been read', async () => {
    // The condition has not gone away, and that is fine: the user has read the
    // sentence and is dealing with it. Putting it back ten seconds later is how
    // an informational banner becomes an alarm.
    present = [peer({ sessionId: 's1', user: 'Nadia', editing: ['kael'] })]
    const qc = await open()
    fireEvent.click(row('Kael'))
    await act(async () => {})

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }))
    expect(bannerCodes()).toEqual([])

    present = [peer({ sessionId: 's1', user: 'Nadia', editing: ['kael'], seenSecsAgo: 30 })]
    await poll(qc)
    expect(bannerCodes()).toEqual([])
  })

  it('goes away when the other session does', async () => {
    present = [peer({ sessionId: 's1', user: 'Nadia', editing: ['kael'] })]
    const qc = await open()
    fireEvent.click(row('Kael'))
    await act(async () => {})
    expect(bannerCodes()).toEqual([PRESENCE_BANNER])

    present = []
    await poll(qc)
    expect(bannerCodes()).toEqual([])
  })

  it('is not raised for a heartbeat that has already gone stale', async () => {
    present = [
      peer({
        sessionId: 's1',
        user: 'Nadia',
        editing: ['kael'],
        seenSecsAgo: STALE_AFTER_SECS + 1,
      }),
    ]
    await open()
    fireEvent.click(row('Kael'))
    await act(async () => {})

    expect(bannerCodes()).toEqual([])
  })

  it('changes nothing about what the user can do', async () => {
    // The whole point. `docs/07-file-shares.md` is explicit that hard locks over
    // a share strand files and that we warn rather than block, so a banner that
    // arrived with a disabled control would be the feature going wrong.
    present = [peer({ sessionId: 's1', user: 'Nadia', editing: ['kael'] })]
    await open()
    fireEvent.click(row('Kael'))
    await act(async () => {})
    expect(bannerCodes()).toEqual([PRESENCE_BANNER])

    const notes = (await screen.findByPlaceholderText(/Write your notes/)) as HTMLTextAreaElement
    expect(notes.readOnly).toBe(false)
    expect((screen.getByRole('button', { name: 'New entity' }) as HTMLButtonElement).disabled).toBe(
      false,
    )
    expect(row('Kael').getAttribute('draggable')).toBe('true')
    expect((await screen.findByLabelText('Entity name')) as HTMLInputElement).toHaveProperty(
      'readOnly',
      false,
    )
  })
})

describe('the status bar', () => {
  it('counts the sessions in the folder, this one included', async () => {
    present = [peer({ sessionId: 's1', user: 'Nadia' }), peer({ sessionId: 's2', user: 'Tomas' })]
    await open()
    expect(screen.getByText('3 people here')).toBeTruthy()
  })

  it('says nothing when this is the only session', async () => {
    await open()
    expect(screen.queryByText(/sessions/)).toBeNull()
  })
})

describe('what this session tells everyone else', () => {
  it('reports the node it has open, and reports letting go of it', async () => {
    await open()
    expect(h.invoke).toHaveBeenCalledWith('presence_editing', { nodeIds: [] })

    fireEvent.click(row('Kael'))
    await act(async () => {})
    expect(h.invoke).toHaveBeenCalledWith('presence_editing', { nodeIds: ['kael'] })

    // Without this, the last node we happened to look at stays marked as ours on
    // everyone else's rows for the rest of the session.
    act(() => useUI.getState().select(null))
    await act(async () => {})
    const sent = h.invoke.mock.calls.filter((c) => c[0] === 'presence_editing')
    expect(sent.at(-1)?.[1]).toEqual({ nodeIds: [] })
  })
})
