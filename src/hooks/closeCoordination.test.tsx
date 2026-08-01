import type { ReactNode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { editorWrites } from '../lib/editorWrites'
import { EDITOR_CLOSE_BLOCKED } from '../lib/projectClose'
import { useCloseProject } from '../lib/queries'
import { useUI } from '../store/ui'
import { node } from '../test/fixtures'
import { useAutosaveNode } from './useAutosaveNode'
import { useSafeWindowClose } from './useSafeWindowClose'

interface CloseEvent {
  preventDefault: () => void
}

const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  closeWindow: vi.fn(() => Promise.resolve()),
  closeHandler: null as null | ((event: CloseEvent) => void | Promise<void>),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))
vi.mock('../lib/window', () => ({
  closeWindow: h.closeWindow,
  onCloseRequested: (handler: (event: CloseEvent) => void | Promise<void>) => {
    h.closeHandler = handler
    return Promise.resolve(() => {
      h.closeHandler = null
    })
  },
}))

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })
  return function TestRoot({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>
  }
}

function useProjectCloseHarness() {
  const autosave = useAutosaveNode(node({ id: 'kael' }), { delay: 60_000 })
  const close = useCloseProject()
  return { autosave, close }
}

function useWindowCloseHarness() {
  const autosave = useAutosaveNode(node({ id: 'kael' }), { delay: 60_000 })
  useSafeWindowClose(true)
  return autosave
}

beforeEach(() => {
  h.invoke.mockReset()
  h.closeWindow.mockClear()
  h.closeHandler = null
  editorWrites.reset()
  useUI.setState({ banners: [], toasts: [] })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
})

afterEach(() => {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

describe('closing with editor writes', () => {
  it('awaits the final node_upsert before project_close', async () => {
    const commands: string[] = []
    let finishSave: (() => void) | undefined
    h.invoke.mockImplementation((command: string, args?: { node?: ReturnType<typeof node> }) => {
      commands.push(command)
      if (command === 'node_upsert') {
        return new Promise((resolve) => {
          finishSave = () => resolve(args?.node)
        })
      }
      return Promise.resolve(null)
    })
    const view = renderHook(useProjectCloseHarness, { wrapper: wrapper() })

    act(() => view.result.current.autosave.queue({ notesRaw: 'the final sentence' }))
    act(() => view.result.current.close.mutate())

    await waitFor(() => expect(commands).toEqual(['node_upsert']))
    await act(async () => finishSave?.())
    await waitFor(() => expect(commands).toEqual(['node_upsert', 'project_close']))
    view.unmount()
  })

  it('intercepts a normal window close during the debounce and preserves the edit', async () => {
    const commands: string[] = []
    let finishSave: (() => void) | undefined
    h.invoke.mockImplementation((command: string, args?: { node?: ReturnType<typeof node> }) => {
      commands.push(command)
      if (command === 'node_upsert') {
        expect(args?.node?.notesRaw).toBe('typed just before close')
        return new Promise((resolve) => {
          finishSave = () => resolve(args?.node)
        })
      }
      return Promise.resolve(null)
    })
    const view = renderHook(useWindowCloseHarness, { wrapper: wrapper() })
    await waitFor(() => expect(h.closeHandler).toBeTypeOf('function'))
    act(() => view.result.current.queue({ notesRaw: 'typed just before close' }))
    expect(editorWrites.snapshot()).toMatchObject([{ nodeId: 'kael', state: 'pending' }])
    const closeEvent = { preventDefault: vi.fn() }

    let closeRequest: void | Promise<void> | undefined
    act(() => {
      closeRequest = h.closeHandler?.(closeEvent)
    })
    expect(closeEvent.preventDefault).toHaveBeenCalledOnce()
    await waitFor(() => expect(commands).toEqual(['node_upsert']))
    expect(h.closeWindow).not.toHaveBeenCalled()

    await act(async () => finishSave?.())
    await act(async () => closeRequest)
    expect(commands).toEqual(['node_upsert', 'project_close'])
    expect(h.closeWindow).toHaveBeenCalledOnce()

    const permittedEvent = { preventDefault: vi.fn() }
    await act(async () => h.closeHandler?.(permittedEvent))
    expect(permittedEvent.preventDefault).not.toHaveBeenCalled()
    view.unmount()
  })

  it('keeps the workspace open after failure and retries the retained patch', async () => {
    const commands: string[] = []
    const rejected = { code: 'node.invalid', message: 'invalid node', retryable: false }
    let fail = true
    h.invoke.mockImplementation((command: string, args?: { node?: ReturnType<typeof node> }) => {
      commands.push(command)
      if (command === 'node_upsert') {
        return fail ? Promise.reject(rejected) : Promise.resolve(args?.node)
      }
      return Promise.resolve(null)
    })
    const view = renderHook(useWindowCloseHarness, { wrapper: wrapper() })
    await waitFor(() => expect(h.closeHandler).toBeTypeOf('function'))
    act(() => view.result.current.queue({ notesRaw: 'do not discard me' }))

    const firstEvent = { preventDefault: vi.fn() }
    await act(async () => h.closeHandler?.(firstEvent))
    expect(commands).toEqual(['node_upsert'])
    expect(h.closeWindow).not.toHaveBeenCalled()
    expect(editorWrites.snapshot()).toMatchObject([{ nodeId: 'kael', state: 'failed' }])
    const banner = useUI.getState().banners.find((item) => item.code === EDITOR_CLOSE_BLOCKED)
    expect(banner).toMatchObject({ sticky: true, action: { label: 'Retry save and close' } })

    fail = false
    act(() => banner?.action?.run())
    expect(h.closeWindow).toHaveBeenCalledOnce()
    await act(async () => h.closeHandler?.({ preventDefault: vi.fn() }))

    expect(commands).toEqual(['node_upsert', 'node_upsert', 'project_close'])
    expect(h.closeWindow).toHaveBeenCalledTimes(2)
    expect(useUI.getState().banners.some((item) => item.code === EDITOR_CLOSE_BLOCKED)).toBe(false)
    view.unmount()
  })
})
