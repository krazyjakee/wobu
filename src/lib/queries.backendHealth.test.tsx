import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook } from '@testing-library/react'
import type { ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { JobDone, JobFailed, QueueSnapshot, StatusBarBackend } from './api'
import { useSetProviderKey, useStatusBarBackend } from './queries'

const h = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, Set<(event: { payload: unknown }) => void>>(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((name: string, handler: (event: { payload: unknown }) => void) => {
    const listeners = h.listeners.get(name) ?? new Set()
    listeners.add(handler)
    h.listeners.set(name, listeners)
    return Promise.resolve(() => listeners.delete(handler))
  }),
}))

const text: StatusBarBackend['text'] = {
  provider: 'anthropic',
  label: 'Anthropic',
  model: 'claude-sonnet-5',
  contextTokens: 1_000_000,
}

function connected(externalQueue: number | null): StatusBarBackend {
  return {
    image: {
      provider: externalQueue === null ? 'gemini' : 'comfyui',
      label: externalQueue === null ? 'Gemini' : 'ComfyUI',
      model: externalQueue === null ? 'gemini-2.5-flash-image' : 'flux-dev',
      contextTokens: null,
    },
    text,
    health: { state: 'connected', externalQueue },
  }
}

const unavailable: StatusBarBackend = {
  ...connected(2),
  health: { state: 'unavailable', detail: 'connection refused' },
}

let answer: StatusBarBackend
let visibility: DocumentVisibilityState
let qc: QueryClient

function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>
}

function statusCalls(): number {
  return h.invoke.mock.calls.filter(([command]) => command === 'status_bar_backend').length
}

async function tick(ms = 0) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms)
  })
}

async function emit<T>(name: string, payload: T) {
  await act(async () => {
    for (const listener of h.listeners.get(name) ?? []) listener({ payload })
    await vi.advanceTimersByTimeAsync(0)
  })
}

beforeEach(() => {
  vi.useFakeTimers()
  h.invoke.mockReset()
  h.listeners.clear()
  answer = connected(null)
  visibility = 'visible'
  vi.spyOn(document, 'visibilityState', 'get').mockImplementation(() => visibility)
  h.invoke.mockImplementation((command: string) => {
    if (command === 'status_bar_backend') return Promise.resolve(answer)
    if (command === 'provider_key_set') return Promise.resolve(undefined)
    throw new Error(`unexpected command: ${command}`)
  })
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  })
})

afterEach(() => {
  qc.clear()
  vi.restoreAllMocks()
  vi.useRealTimers()
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
})

describe('idle backend health', () => {
  it('does not poll Gemini, whose backend has no live external queue', async () => {
    renderHook(() => useStatusBarBackend('ashfall'), { wrapper })
    await tick()
    expect(statusCalls()).toBe(1)

    await tick(5 * 60_000)
    expect(statusCalls()).toBe(1)
  })

  it('preserves an explicit refresh while automatic polling is stopped', async () => {
    const { result } = renderHook(() => useStatusBarBackend('ashfall'), { wrapper })
    await tick()
    await tick(60_000)
    expect(statusCalls()).toBe(1)

    await act(async () => {
      await result.current.refetch()
    })
    expect(statusCalls()).toBe(2)
  })
})

describe('queue-aware polling', () => {
  it('polls only for a live external queue and backs off repeated failures', async () => {
    answer = connected(2)
    renderHook(() => useStatusBarBackend('ashfall'), { wrapper })
    await tick()
    expect(statusCalls()).toBe(1)

    answer = unavailable
    await tick(5_000)
    expect(statusCalls()).toBe(2)
    await tick(5_000)
    expect(statusCalls()).toBe(3)

    // The second consecutive failure doubles the next interval.
    await tick(9_999)
    expect(statusCalls()).toBe(3)
    answer = connected(0)
    await tick(1)
    expect(statusCalls()).toBe(4)

    // Recovery to an idle queue ends polling rather than returning to 30 s.
    await tick(5 * 60_000)
    expect(statusCalls()).toBe(4)
  })

  it('pauses queue polling while the document is hidden', async () => {
    answer = connected(1)
    renderHook(() => useStatusBarBackend('ashfall'), { wrapper })
    await tick()

    visibility = 'hidden'
    document.dispatchEvent(new Event('visibilitychange'))
    await tick(30_000)
    expect(statusCalls()).toBe(1)

    visibility = 'visible'
    document.dispatchEvent(new Event('visibilitychange'))
    await tick(4_999)
    expect(statusCalls()).toBe(1)
    await tick(1)
    expect(statusCalls()).toBe(2)
  })
})

describe('meaningful health refresh signals', () => {
  it('updates after an actual generation failure and subsequent recovery', async () => {
    const { result } = renderHook(() => useStatusBarBackend('ashfall'), { wrapper })
    await tick()

    answer = unavailable
    await emit<JobFailed>('job:error', {
      id: 'job-1',
      kind: 'generate',
      label: 'Generate Kael',
      failure: {
        code: 'provider.unavailable',
        message: 'offline',
        retryable: true,
        billed: 'nothing',
      },
      retryHeld: false,
    })
    expect(statusCalls()).toBe(2)
    expect(result.current.data?.health.state).toBe('unavailable')

    answer = connected(null)
    await emit<JobDone>('job:done', {
      id: 'job-2',
      kind: 'generate',
      label: 'Generate Kael',
    })
    expect(statusCalls()).toBe(3)
    expect(result.current.data?.health.state).toBe('connected')
  })

  it('refreshes once when a generation enters a real provider attempt', async () => {
    renderHook(() => useStatusBarBackend('ashfall'), { wrapper })
    await tick()
    const snapshot: QueueSnapshot = {
      queued: 0,
      running: 1,
      retrying: 0,
      jobs: [
        {
          id: 'job-1',
          kind: 'generate',
          label: 'Generate Kael',
          subjectId: 'kael',
          state: 'running',
          attempt: 1,
          elapsedMs: 10,
        },
      ],
    }

    await emit('job:state', snapshot)
    expect(statusCalls()).toBe(2)
    await emit('job:state', snapshot)
    expect(statusCalls()).toBe(2)
  })

  it('refreshes after a key change and on network reconnect', async () => {
    const { result } = renderHook(
      () => ({ health: useStatusBarBackend('ashfall'), key: useSetProviderKey() }),
      { wrapper },
    )
    await tick()

    await act(async () => {
      await result.current.key.save('gemini', 'secret')
      await vi.advanceTimersByTimeAsync(0)
    })
    expect(statusCalls()).toBe(2)

    window.dispatchEvent(new Event('online'))
    await tick()
    expect(statusCalls()).toBe(3)
  })
})
