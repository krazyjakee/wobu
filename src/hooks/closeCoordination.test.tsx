import type { ReactNode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { JobSnapshot, QueueSnapshot } from '../lib/api'
import { editorWrites } from '../lib/editorWrites'
import { EDITOR_CLOSE_BLOCKED, JOBS_CLOSE_BLOCKED } from '../lib/projectClose'
import { useCloseProject } from '../lib/queries'
import { useUI } from '../store/ui'
import { node } from '../test/fixtures'
import { useAutosaveNode } from './useAutosaveNode'
import { useSafeWindowClose } from './useSafeWindowClose'

interface CloseEvent {
  preventDefault: () => void
}

/** A queue as `job_list` reports it, counts included. */
function queue(...jobs: JobSnapshot[]): QueueSnapshot {
  const count = (state: JobSnapshot['state']) => jobs.filter((job) => job.state === state).length
  return {
    jobs,
    queued: count('queued'),
    running: count('running'),
    retrying: count('retrying'),
  }
}

function job(id: string, state: JobSnapshot['state'], label = `job ${id}`): JobSnapshot {
  return {
    id,
    kind: 'generate',
    label,
    subjectId: null,
    attempt: 1,
    elapsedMs: 0,
    state,
  } as JobSnapshot
}

const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  closeWindow: vi.fn(() => Promise.resolve()),
  destroyWindow: vi.fn(() => Promise.resolve()),
  closeHandler: null as null | ((event: CloseEvent) => void | Promise<void>),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))
vi.mock('../lib/window', () => ({
  closeWindow: h.closeWindow,
  destroyWindow: h.destroyWindow,
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

/** Quitting from the launcher: no project, but the queue is still there. */
function useLauncherCloseHarness() {
  useSafeWindowClose(false)
}

beforeEach(() => {
  h.invoke.mockReset()
  h.closeWindow.mockClear()
  h.destroyWindow.mockClear()
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
    expect(h.destroyWindow).not.toHaveBeenCalled()

    await act(async () => finishSave?.())
    await act(async () => closeRequest)
    // The queue is consulted between the save landing and the project closing:
    // after, so a warning the user declines has already kept their typing, and
    // before, so declining does not leave them in the launcher.
    expect(commands).toEqual(['node_upsert', 'job_list', 'project_close'])
    expect(h.destroyWindow).toHaveBeenCalledOnce()

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
    expect(h.destroyWindow).not.toHaveBeenCalled()
    expect(editorWrites.snapshot()).toMatchObject([{ nodeId: 'kael', state: 'failed' }])
    const banner = useUI.getState().banners.find((item) => item.code === EDITOR_CLOSE_BLOCKED)
    expect(banner).toMatchObject({ sticky: true, action: { label: 'Retry save and close' } })

    fail = false
    act(() => banner?.action?.run())
    expect(h.closeWindow).toHaveBeenCalledOnce()
    await act(async () => h.closeHandler?.({ preventDefault: vi.fn() }))

    expect(commands).toEqual(['node_upsert', 'node_upsert', 'job_list', 'project_close'])
    expect(h.destroyWindow).toHaveBeenCalledOnce()
    expect(useUI.getState().banners.some((item) => item.code === EDITOR_CLOSE_BLOCKED)).toBe(false)
    view.unmount()
  })
})

describe('closing with jobs in flight', () => {
  /** `job_list` answers with `snapshot`; everything else succeeds silently. */
  function backend(snapshot: QueueSnapshot, commands: string[]) {
    h.invoke.mockImplementation((command: string) => {
      commands.push(command)
      if (command === 'job_list') return Promise.resolve(snapshot)
      return Promise.resolve(null)
    })
  }

  it('refuses the first quit and says what it would destroy', async () => {
    // The acceptance criterion this exists for: a generation the user has paid
    // for is work in progress, and taking it away without asking is the same
    // class of mistake as dropping an unsaved paragraph.
    const commands: string[] = []
    backend(queue(job('01J', 'running', 'Generate Kael')), commands)
    const view = renderHook(useWindowCloseHarness, { wrapper: wrapper() })
    await waitFor(() => expect(h.closeHandler).toBeTypeOf('function'))

    const event = { preventDefault: vi.fn() }
    await act(async () => h.closeHandler?.(event))

    expect(event.preventDefault).toHaveBeenCalledOnce()
    expect(h.destroyWindow).not.toHaveBeenCalled()
    expect(commands).toEqual(['job_list'])
    expect(commands).not.toContain('project_close')
    const banner = useUI.getState().banners.find((item) => item.code === JOBS_CLOSE_BLOCKED)
    expect(banner).toMatchObject({
      detail: 'Generate Kael',
      action: { label: 'Stop them and quit' },
    })
    expect(banner?.sticky).toBeFalsy()
    view.unmount()
  })

  it('stops every unfinished job before it lets the window go', async () => {
    // Cancelling rather than letting the process take them is the whole point:
    // it is the only path on which an adapter gets to report what it was billed
    // and a ComfyUI run gets its `/interrupt`.
    const commands: string[] = []
    const cancelled: unknown[] = []
    const snapshot = queue(job('01J', 'running'), job('02K', 'queued'), job('03L', 'done'))
    h.invoke.mockImplementation((command: string, args?: { jobId?: string }) => {
      commands.push(command)
      if (command === 'job_list') return Promise.resolve(snapshot)
      if (command === 'job_cancel') cancelled.push(args?.jobId)
      return Promise.resolve(null)
    })
    const view = renderHook(useWindowCloseHarness, { wrapper: wrapper() })
    await waitFor(() => expect(h.closeHandler).toBeTypeOf('function'))

    await act(async () => h.closeHandler?.({ preventDefault: vi.fn() }))
    const banner = useUI.getState().banners.find((item) => item.code === JOBS_CLOSE_BLOCKED)
    act(() => banner?.action?.run())
    expect(h.closeWindow).toHaveBeenCalledOnce()
    await act(async () => h.closeHandler?.({ preventDefault: vi.fn() }))

    expect(cancelled).toEqual(['01J', '02K'])
    expect(commands.filter((command) => command === 'job_cancel')).toHaveLength(2)
    expect(commands).toContain('project_close')
    expect(commands.indexOf('project_close')).toBeGreaterThan(commands.indexOf('job_cancel'))
    expect(h.destroyWindow).toHaveBeenCalledOnce()
    expect(useUI.getState().banners.some((item) => item.code === JOBS_CLOSE_BLOCKED)).toBe(false)
    view.unmount()
  })

  it('does not hold the quit for jobs that have already finished', async () => {
    // `job_list` keeps a tail of finished jobs so the status bar can show the
    // last outcome. Counting those would make every quit after a generation a
    // question with no content.
    const commands: string[] = []
    backend(queue(job('01J', 'done'), job('02K', 'cancelled'), job('03L', 'failed')), commands)
    const view = renderHook(useWindowCloseHarness, { wrapper: wrapper() })
    await waitFor(() => expect(h.closeHandler).toBeTypeOf('function'))

    await act(async () => h.closeHandler?.({ preventDefault: vi.fn() }))

    expect(commands).toEqual(['job_list', 'project_close'])
    expect(h.destroyWindow).toHaveBeenCalledOnce()
    expect(useUI.getState().banners).toEqual([])
    view.unmount()
  })

  it('guards a quit from the launcher, where no project is open at all', async () => {
    // The queue is per installation and outlives the project it was started
    // for, so "nothing is open" is not "nothing is running" — and a gate that
    // only ran with a project open would miss it entirely.
    const commands: string[] = []
    backend(queue(job('01J', 'running', 'Generate Vashk')), commands)
    const view = renderHook(useLauncherCloseHarness, { wrapper: wrapper() })
    await waitFor(() => expect(h.closeHandler).toBeTypeOf('function'))

    await act(async () => h.closeHandler?.({ preventDefault: vi.fn() }))
    expect(h.destroyWindow).not.toHaveBeenCalled()
    const banner = useUI.getState().banners.find((item) => item.code === JOBS_CLOSE_BLOCKED)
    expect(banner?.detail).toBe('Generate Vashk')

    act(() => banner?.action?.run())
    await act(async () => h.closeHandler?.({ preventDefault: vi.fn() }))

    expect(commands).toContain('job_cancel')
    expect(commands).not.toContain('project_close')
    expect(h.destroyWindow).toHaveBeenCalledOnce()
    view.unmount()
  })

  it('quits rather than hanging when the queue cannot be read', async () => {
    // A backend that cannot answer must not become a window that will not
    // close. The Rust wind-down cancels whatever is left regardless; all that
    // is lost here is the chance to warn.
    const commands: string[] = []
    h.invoke.mockImplementation((command: string) => {
      commands.push(command)
      if (command === 'job_list') return Promise.reject(new Error('the backend is gone'))
      return Promise.resolve(null)
    })
    const view = renderHook(useWindowCloseHarness, { wrapper: wrapper() })
    await waitFor(() => expect(h.closeHandler).toBeTypeOf('function'))

    await act(async () => h.closeHandler?.({ preventDefault: vi.fn() }))

    expect(commands).toEqual(['job_list', 'project_close'])
    expect(h.destroyWindow).toHaveBeenCalledOnce()
    view.unmount()
  })
})
