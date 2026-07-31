import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { saveLabel, useAutosaveNode } from './useAutosaveNode'
import type { WobuNode } from '../lib/api'
import { node } from '../test/fixtures'
import { useUI } from '../store/ui'
import { useSettings } from '../store/settings'

/*
 * This hook is the last thing standing between a keystroke and the disk, and
 * every failure mode it has is silent: an edit that never leaves the debounce
 * window, a save that overwrites newer text with older, a patch dropped on a
 * transient error. None of those show up on screen — they show up a week later
 * as a paragraph the user is sure they wrote. So the assertions below are about
 * bytes reaching `node_upsert`, not about status strings.
 */

interface MutateOpts {
  onSuccess: (saved: WobuNode) => void
  onError: (e: unknown) => void
}

const h = vi.hoisted(() => ({
  mutate: vi.fn(),
  listeners: new Map<string, () => void>(),
}))

vi.mock('../lib/queries', () => ({
  useUpsertNode: () => ({ mutate: h.mutate }),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, cb: () => void) => {
    h.listeners.set(name, cb)
    return Promise.resolve(() => h.listeners.delete(name))
  },
}))

/** What was actually handed to `node_upsert`, in order. */
const sent = () => h.mutate.mock.calls.map((c) => c[0] as WobuNode)

/** Resolve the `listen()` promise so the share:online handler is registered. */
async function render(n: WobuNode | undefined, delay = 500) {
  const r = renderHook(({ node }: { node: WobuNode | undefined }) => useAutosaveNode(node, delay), {
    initialProps: { node: n },
  })
  await act(async () => {})
  return r
}

/** Exactly how the editor calls it: no delay argument at all. */
async function renderAsEditorDoes(n: WobuNode) {
  const r = renderHook(({ node }: { node: WobuNode }) => useAutosaveNode(node), {
    initialProps: { node: n },
  })
  await act(async () => {})
  return r
}

beforeEach(() => {
  vi.useFakeTimers()
  h.mutate.mockReset()
  h.listeners.clear()
  useUI.setState({ toasts: [], banners: [] })
  useSettings.getState().reset()
  // Without this `isTauri()` is false and the share:online listener never
  // attaches — which is exactly right in a browser, and useless in a test.
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

afterEach(() => {
  vi.useRealTimers()
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

describe('debouncing', () => {
  it('holds the write until the window closes', async () => {
    const { result } = await render(node({ id: 'a' }))
    act(() => result.current.queue({ notesRaw: 'hello' }))

    expect(result.current.status).toBe('dirty')
    act(() => void vi.advanceTimersByTime(499))
    expect(h.mutate).not.toHaveBeenCalled()

    act(() => void vi.advanceTimersByTime(1))
    expect(sent()).toHaveLength(1)
    expect(sent()[0]!.notesRaw).toBe('hello')
  })

  it('coalesces a burst of keystrokes into one write', async () => {
    const { result } = await render(node({ id: 'a' }))
    for (const text of ['h', 'he', 'hel', 'hell', 'hello']) {
      act(() => result.current.queue({ notesRaw: text }))
      act(() => void vi.advanceTimersByTime(100))
    }
    act(() => void vi.advanceTimersByTime(500))

    expect(sent()).toHaveLength(1)
    expect(sent()[0]!.notesRaw).toBe('hello')
  })

  it('merges fields edited in separate keystrokes into one node', async () => {
    const { result } = await render(node({ id: 'a' }))
    act(() => result.current.queue({ name: 'Kell' }))
    act(() => result.current.queue({ summary: 'an ashwalker' }))
    act(() => void vi.advanceTimersByTime(500))

    expect(sent()[0]).toMatchObject({ id: 'a', name: 'Kell', summary: 'an ashwalker' })
  })

  it('uses the configured delay when the caller does not pass one', async () => {
    // The editor calls `useAutosaveNode(node)` with no delay, so this is the
    // path that actually runs in the app — the explicit argument above exists
    // for the tests. Setting it in Settings has to reach the open pane.
    useSettings.getState().setAutosaveDelay(1500)
    const { result } = await renderAsEditorDoes(node({ id: 'a' }))
    act(() => result.current.queue({ notesRaw: 'slow' }))

    act(() => void vi.advanceTimersByTime(1400))
    expect(h.mutate).not.toHaveBeenCalled()
    act(() => void vi.advanceTimersByTime(100))
    expect(sent()).toHaveLength(1)
  })

  it('sends nothing when nothing was queued', async () => {
    const { result } = await render(node({ id: 'a' }))
    act(() => result.current.flush())
    expect(h.mutate).not.toHaveBeenCalled()
  })
})

describe('flushing', () => {
  it('flush() writes immediately and cancels the pending timer', async () => {
    const { result } = await render(node({ id: 'a' }))
    act(() => result.current.queue({ notesRaw: 'hello' }))
    act(() => result.current.flush())
    expect(sent()).toHaveLength(1)

    // The timer must not fire a second, redundant write.
    act(() => void vi.advanceTimersByTime(1000))
    expect(sent()).toHaveLength(1)
  })

  it('flushes on unmount — closing the pane must not eat the last sentence', async () => {
    const { result, unmount } = await render(node({ id: 'a' }))
    act(() => result.current.queue({ notesRaw: 'the last sentence' }))
    expect(h.mutate).not.toHaveBeenCalled()

    unmount()
    expect(sent()[0]!.notesRaw).toBe('the last sentence')
  })

  it('flushes when the edited node is swapped out', async () => {
    const { result, rerender } = await render(node({ id: 'a' }))
    act(() => result.current.queue({ notesRaw: 'about a' }))

    act(() => rerender({ node: node({ id: 'b' }) }))

    // Crucially the write must still carry a's id — selecting another node
    // mid-debounce is how an edit ends up on the wrong file.
    expect(sent()).toHaveLength(1)
    expect(sent()[0]!.id).toBe('a')
    expect(sent()[0]!.notesRaw).toBe('about a')
  })

  it('does not write on unmount when there was nothing pending', async () => {
    const { unmount } = await render(node({ id: 'a' }))
    unmount()
    expect(h.mutate).not.toHaveBeenCalled()
  })
})

describe('which node the patch is merged onto', () => {
  it('ignores a server refetch that arrives while an edit is pending', async () => {
    // `world:changed` fires on our own writes too, so a refetch landing
    // mid-typing is routine. Adopting it would send the server's older text
    // back as if the user had typed it.
    const { result, rerender } = await render(node({ id: 'a', summary: 'original' }))
    act(() => result.current.queue({ notesRaw: 'typing' }))
    act(() => rerender({ node: node({ id: 'a', summary: 'from the watcher' }) }))
    act(() => void vi.advanceTimersByTime(500))

    expect(sent()[0]!.summary).toBe('original')
    expect(sent()[0]!.notesRaw).toBe('typing')
  })

  it('adopts a server refetch when nothing is pending', async () => {
    const { result, rerender } = await render(node({ id: 'a', summary: 'original' }))
    act(() => rerender({ node: node({ id: 'a', summary: 'edited in Obsidian' }) }))
    act(() => result.current.queue({ notesRaw: 'typing' }))
    act(() => void vi.advanceTimersByTime(500))

    expect(sent()[0]!.summary).toBe('edited in Obsidian')
  })

  it('builds the next write on what the backend actually saved', async () => {
    // The backend normalises — slug from name, updatedAt. The second write has
    // to start from that, or it reverts fields it never touched.
    //
    // The rerender is not test scaffolding, it is the mechanism: `useUpsertNode`
    // runs `setQueryData` before this callback, so by the time the hook
    // re-renders the prop already *is* the saved node. `latest.current = saved`
    // inside onSuccess only bridges the gap within the tick.
    const saved = { ...node({ id: 'a' }), name: 'Kell', slug: 'kell' }
    h.mutate.mockImplementation((_n: WobuNode, o: MutateOpts) => o.onSuccess(saved))

    const { result, rerender } = await render(node({ id: 'a' }))
    act(() => result.current.queue({ name: 'Kell' }))
    act(() => void vi.advanceTimersByTime(500))
    expect(result.current.status).toBe('saved')
    act(() => rerender({ node: saved }))

    h.mutate.mockImplementation(() => {})
    act(() => result.current.queue({ summary: 'later' }))
    act(() => void vi.advanceTimersByTime(500))
    expect(sent()[1]).toMatchObject({ name: 'Kell', slug: 'kell', summary: 'later' })
  })

  it('does not let a save landing mid-typing reset what is being typed onto', async () => {
    // A slow write completing after the user has started the next edit. The
    // guard on `patch.current` is what stops the acknowledged node replacing
    // the base that the newer keystrokes are already merged against.
    let ack: (() => void) | undefined
    h.mutate.mockImplementation((_n: WobuNode, o: MutateOpts) => {
      ack = () => o.onSuccess({ ...node({ id: 'a' }), summary: 'server wording' })
    })

    const { result } = await render(node({ id: 'a', summary: 'mine' }))
    act(() => result.current.queue({ notesRaw: 'one' }))
    act(() => void vi.advanceTimersByTime(500))

    act(() => result.current.queue({ notesRaw: 'two' }))
    act(() => ack?.())
    act(() => void vi.advanceTimersByTime(500))

    expect(sent()[1]!.summary).toBe('mine')
    expect(sent()[1]!.notesRaw).toBe('two')
  })
})

describe('a save that fails', () => {
  const unreachable = { code: 'share.unmounted', message: 'the share is away', retryable: true }
  const rejected = { code: 'node.invalid', message: 'a name is required', retryable: false }

  it('holds a retryable failure and resends the same text later', async () => {
    h.mutate.mockImplementation((_n: WobuNode, o: MutateOpts) => o.onError(unreachable))
    const { result } = await render(node({ id: 'a' }))
    act(() => result.current.queue({ notesRaw: 'written while offline' }))
    act(() => void vi.advanceTimersByTime(500))
    expect(result.current.status).toBe('held')

    h.mutate.mockImplementation(() => {})
    act(() => result.current.flush())
    expect(sent()[1]!.notesRaw).toBe('written while offline')
  })

  it('lets newer keystrokes win over the text it put back', async () => {
    h.mutate.mockImplementation((_n: WobuNode, o: MutateOpts) => o.onError(unreachable))
    const { result } = await render(node({ id: 'a' }))
    act(() => result.current.queue({ notesRaw: 'first' }))
    act(() => void vi.advanceTimersByTime(500))

    h.mutate.mockImplementation(() => {})
    act(() => result.current.queue({ notesRaw: 'second' }))
    act(() => void vi.advanceTimersByTime(500))
    expect(sent()[1]!.notesRaw).toBe('second')
  })

  it('drops a patch the backend genuinely refused', async () => {
    // Resending something already rejected just fails again, forever.
    h.mutate.mockImplementation((_n: WobuNode, o: MutateOpts) => o.onError(rejected))
    const { result } = await render(node({ id: 'a' }))
    act(() => result.current.queue({ name: '' }))
    act(() => void vi.advanceTimersByTime(500))
    expect(result.current.status).toBe('error')

    h.mutate.mockImplementation(() => {})
    act(() => result.current.flush())
    expect(sent()).toHaveLength(1)
  })

  it('reports the failure on the surface its code calls for', async () => {
    h.mutate.mockImplementation((_n: WobuNode, o: MutateOpts) => o.onError(unreachable))
    const { result } = await render(node({ id: 'a' }))
    act(() => result.current.queue({ notesRaw: 'x' }))
    act(() => void vi.advanceTimersByTime(500))

    expect(useUI.getState().banners.map((b) => b.code)).toEqual(['share.unmounted'])
    expect(useUI.getState().toasts).toEqual([])
  })

  it('resends when the share comes back, without another keystroke', async () => {
    h.mutate.mockImplementation((_n: WobuNode, o: MutateOpts) => o.onError(unreachable))
    const { result } = await render(node({ id: 'a' }))
    act(() => result.current.queue({ notesRaw: 'held text' }))
    act(() => void vi.advanceTimersByTime(500))
    expect(result.current.status).toBe('held')

    h.mutate.mockImplementation((n: WobuNode, o: MutateOpts) => o.onSuccess(n))
    act(() => h.listeners.get('share:online')?.())

    expect(sent()[1]!.notesRaw).toBe('held text')
    expect(result.current.status).toBe('saved')
  })

  it('the share coming back with nothing held writes nothing', async () => {
    const { result } = await render(node({ id: 'a' }))
    act(() => h.listeners.get('share:online')?.())
    expect(h.mutate).not.toHaveBeenCalled()
    expect(result.current.status).toBe('idle')
  })
})

describe('no node yet', () => {
  it('queues without writing, because there is nothing to merge onto', async () => {
    const { result } = await render(undefined)
    act(() => result.current.queue({ notesRaw: 'x' }))
    act(() => void vi.advanceTimersByTime(500))
    expect(h.mutate).not.toHaveBeenCalled()
  })
})

describe('saveLabel', () => {
  it('says what is happening, and says nothing when nothing is', () => {
    expect(saveLabel('idle')).toBe('')
    expect(saveLabel('dirty')).toBe('unsaved…')
    expect(saveLabel('saving')).toBe('saving…')
    expect(saveLabel('saved')).toBe('saved')
    expect(saveLabel('error')).toBe('save failed')
  })

  it('names the actual reason when an edit is being held', () => {
    // "save failed" here would be a lie — nothing was lost and it will retry.
    expect(saveLabel('held')).toBe('waiting for the share…')
  })
})
