import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { PreviewFrame } from '../../lib/api/narrativePreview'
import { useUI } from '../../store/ui'
import { NarrativePreviewPane } from './NarrativePreviewPane'
import { usePreviewSessions } from './previewStore'

const h = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))
const line: PreviewFrame = {
  trace: { committed: true, error: null, omitted: 0, records: [] },
  snapshot: { at: 'line' },
  current: {
    line: {
      scene: 'scene',
      beat: 'beat',
      slot: 'slot',
      variant: 'variant',
      speaker: 'player',
      text: 'I have proof.',
      revision: 'revision',
    },
  },
  state: { trust: 40 },
}
const choices: PreviewFrame = {
  trace: { committed: true, error: null, omitted: 0, records: [] },
  snapshot: { at: 'choices' },
  current: {
    choices: { scene: 'scene', beat: 'beat', choices: [{ id: 'choice', label: 'Show proof' }] },
  },
  state: { trust: 40 },
}
const command: PreviewFrame = {
  trace: { committed: true, error: null, omitted: 0, records: [] },
  snapshot: { at: 'command' },
  current: { game_command: { token: 'token', name: 'award_badge', args: [true] } },
  state: { trust: 50 },
}
const end: PreviewFrame = {
  trace: { committed: true, error: null, omitted: 0, records: [] },
  snapshot: { at: 'end' },
  current: { end: { label: 'Supported' } },
  state: { trust: 50 },
}
function mount(projectKey = 'project') {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <NarrativePreviewPane projectKey={projectKey} />
    </QueryClientProvider>,
  )
}
beforeEach(() => {
  ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
  usePreviewSessions.setState({ sessions: {}, diagnostics: {}, busy: {} })
  useUI.getState().selectNarrative({ sceneId: 'scene', beatId: 'beat' }, 'script')
  useUI.getState().setNarrativeTab('preview')
  h.invoke.mockReset()
  h.invoke.mockImplementation(async (name: string, args: Record<string, unknown>) => {
    if (name === 'narrative_state_get')
      return {
        document: {
          schema_version: 1,
          variables: [{ name: 'trust', type: { int: { min: 0, max: 100 } }, default: 40 }],
        },
        stamp: null,
      }
    if (name === 'node_list') return []
    if (name === 'narrative_compile') return { graph: { fingerprint: 'build' }, diagnostics: [] }
    if (name === 'narrative_preview_start') return line
    if (name === 'narrative_preview_step') {
      const action = args.action as { kind: string }
      if (action.kind === 'advance') return choices
      if (action.kind === 'choose') return command
      if (action.kind === 'completeCommand') return end
      if (action.kind === 'restore') return line
    }
    throw new Error(`Unexpected ${name}`)
  })
})
describe('deterministic narrative preview', () => {
  it('sets starting state, follows choices, manually acknowledges commands and restores a snapshot', async () => {
    mount()
    fireEvent.change(await screen.findByLabelText('Initial trust'), { target: { value: '71' } })
    fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
    expect(await screen.findByText('I have proof.')).toBeInTheDocument()
    expect(h.invoke).toHaveBeenCalledWith('narrative_preview_start', {
      graph: { fingerprint: 'build' },
      sceneId: 'scene',
      initialState: { trust: 71 },
      seed: 0,
    })
    fireEvent.click(screen.getByRole('button', { name: 'Save snapshot' }))
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Show proof' }))
    expect(await screen.findByText('Host command: award_badge')).toBeInTheDocument()
    expect(screen.getByText('50')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Acknowledge command' }))
    expect(await screen.findByRole('status')).toHaveTextContent('Scene ended: Supported')
    expect(h.invoke).toHaveBeenCalledWith(
      'narrative_preview_step',
      expect.objectContaining({
        action: {
          kind: 'completeCommand',
          token: 'token',
          result: { success: { host_inputs: {} } },
        },
      }),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Restore snapshot' }))
    expect(await screen.findByText('I have proof.')).toBeInTheDocument()
    expect(h.invoke).toHaveBeenCalledWith(
      'narrative_preview_step',
      expect.objectContaining({ snapshot: { at: 'line' }, action: { kind: 'restore' } }),
    )
    expect(usePreviewSessions.getState().sessions['project:scene']?.trace).toHaveLength(5)
  })

  it('retains playback across tabs, isolates projects and links line identity to Script', async () => {
    const view = mount()
    await screen.findByLabelText('Initial trust')
    fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
    await screen.findByText('I have proof.')
    view.unmount()
    const again = mount()
    expect(screen.getByText('I have proof.')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Open this line in Script' }))
    expect(useUI.getState().narrative).toMatchObject({
      sceneId: 'scene',
      beatId: 'beat',
      lineId: 'slot',
    })
    expect(useUI.getState().narrativeTab).toBe('script')
    again.unmount()
    mount('other-project')
    expect(screen.queryByText('I have proof.')).not.toBeInTheDocument()
  })

  it('blocks compilation failures and links diagnostics to their scene and beat', async () => {
    const prior = h.invoke.getMockImplementation()!
    h.invoke.mockImplementation((name, args) =>
      name === 'narrative_compile'
        ? Promise.resolve({
            graph: null,
            diagnostics: [
              {
                scene: 'other-scene',
                site: { variant: { beat: 'bad-beat', slot: 'slot', variant: 'variant' } },
                severity: 'error',
                code: 'type_error',
                message: 'Unknown trust variable',
              },
            ],
          })
        : prior(name, args),
    )
    mount()
    await screen.findByLabelText('Initial trust')
    fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
    fireEvent.click(await screen.findByRole('button', { name: 'error: Unknown trust variable' }))
    expect(useUI.getState().narrative).toMatchObject({ sceneId: 'other-scene', beatId: 'bad-beat' })
    expect(h.invoke.mock.calls.some(([name]) => name === 'narrative_preview_start')).toBe(false)
  })

  it('keeps a failed step and serializes operations across a tab remount', async () => {
    let reject: ((error: Error) => void) | undefined
    const prior = h.invoke.getMockImplementation()!
    h.invoke.mockImplementation((name, args) =>
      name === 'narrative_preview_step'
        ? new Promise((_, fail) => {
            reject = fail
          })
        : prior(name, args),
    )
    const view = mount()
    await screen.findByLabelText('Initial trust')
    fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Continue' }))
    await waitFor(() => expect(reject).toBeDefined())
    view.unmount()
    mount()
    expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled()
    await act(async () =>
      reject?.({ code: 'node.invalid', message: 'Invalid choice state' } as unknown as Error),
    )
    expect(screen.getByText('I have proof.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Continue' })).toBeEnabled()
  })

  it('requires valid explicit host signatures before starting', async () => {
    mount()
    await screen.findByLabelText('Initial trust')
    fireEvent.change(screen.getByLabelText('Command signatures (JSON)'), {
      target: { value: '{"award.badge":["bool"]}' },
    })
    expect(screen.getByRole('button', { name: 'Start preview' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText('Command signatures (JSON)'), {
      target: { value: '{"award_badge":[{"enum":{"members":["true"]}}]}' },
    })
    expect(screen.getByRole('button', { name: 'Start preview' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText('Command signatures (JSON)'), {
      target: { value: '{"award_badge":["bool"]}' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
    await screen.findByText('I have proof.')
    expect(h.invoke).toHaveBeenCalledWith('narrative_compile', {
      commands: { award_badge: ['bool'] },
    })
    fireEvent.change(screen.getByLabelText('Command signatures (JSON)'), {
      target: { value: '{"award_badge":["float"]}' },
    })
    expect(screen.getByRole('button', { name: 'Restart preview' })).toBeDisabled()
    expect(screen.getByRole('alert')).toHaveTextContent('Argument types')
  })
  it('keeps a restored command pending after failure/cancellation and sends typed host results', async () => {
    const pending: PreviewFrame = { ...command, state: { trust: 50, ready: false } }
    usePreviewSessions.getState().put('project:scene', {
      graph: { state: { ready: { ty: 'bool', default: false, owner: 'host' } } },
      frame: pending,
      trace: [],
    })
    const prior = h.invoke.getMockImplementation()!
    h.invoke.mockImplementation((name, args) => {
      if (name !== 'narrative_preview_step') return prior(name, args)
      const action = args.action as { kind: string; result?: unknown }
      if (action.kind === 'restore') return Promise.resolve(pending)
      if (
        action.result === 'cancelled' ||
        (action.result && typeof action.result === 'object' && 'failed' in action.result)
      )
        return Promise.resolve({
          ...pending,
          trace: {
            committed: false,
            error: 'Host command remains pending',
            omitted: 0,
            records: [],
          },
        })
      return Promise.resolve({ ...end, state: { trust: 50, ready: true } })
    })
    const view = mount()
    expect(screen.queryByLabelText('Host output trust')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Save snapshot' }))
    fireEvent.click(screen.getByRole('button', { name: 'Fail command' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Host command remains pending')
    expect(usePreviewSessions.getState().sessions['project:scene']?.frame.snapshot).toEqual(
      command.snapshot,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Cancel command' }))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith(
        'narrative_preview_step',
        expect.objectContaining({
          action: { kind: 'completeCommand', token: 'token', result: 'cancelled' },
        }),
      ),
    )
    view.unmount()
    mount()
    fireEvent.change(screen.getByLabelText('Host output ready'), { target: { value: 'true' } })
    fireEvent.click(screen.getByRole('button', { name: 'Acknowledge command' }))
    expect(await screen.findByRole('status')).toHaveTextContent('Supported')
    expect(h.invoke).toHaveBeenCalledWith(
      'narrative_preview_step',
      expect.objectContaining({
        action: {
          kind: 'completeCommand',
          token: 'token',
          result: { success: { host_inputs: { ready: true } } },
        },
      }),
    )
    fireEvent.click(screen.getByRole('button', { name: 'Restore snapshot' }))
    expect(await screen.findByText('Host command: award_badge')).toBeInTheDocument()
    expect(usePreviewSessions.getState().sessions['project:scene']?.frame.snapshot).toEqual(
      command.snapshot,
    )
  })

  it('shows runtime predicate/effect evidence and links its stable source identity', () => {
    const site = {
      scene: 'scene',
      beat: 'beat',
      slot: null,
      variant: null,
      choice: 'choice',
      outcome: null,
    }
    usePreviewSessions.getState().put('project:scene', {
      graph: {},
      frame: line,
      trace: [
        {
          label: 'Evaluated routes',
          execution: {
            committed: true,
            error: null,
            omitted: 0,
            records: [
              {
                site,
                event: {
                  kind: 'condition',
                  path: [],
                  expression: 'never',
                  passed: false,
                  inputs: {},
                },
              },
              {
                site,
                event: {
                  kind: 'effect',
                  index: 0,
                  effect: { add: { var: 'trust', by: 10 } },
                  before: { trust: 40 },
                  after: { trust: 50 },
                },
              },
            ],
          },
        },
      ],
    })
    mount()
    fireEvent.click(screen.getByText(/Playback trace/))
    expect(screen.getByText('Condition failed')).toBeInTheDocument()
    expect(screen.getByText('Effect 1: values before → after')).toBeInTheDocument()
    fireEvent.click(screen.getAllByRole('button', { name: 'Open choice in Script' })[0]!)
    expect(useUI.getState().narrative).toMatchObject({ sceneId: 'scene', beatId: 'beat' })
    expect(useUI.getState().narrativeTab).toBe('script')
  })
  it('displays the native command error message rather than an object label', async () => {
    const prior = h.invoke.getMockImplementation()!
    h.invoke.mockImplementation((name, args) =>
      name === 'narrative_preview_step'
        ? Promise.reject({
            code: 'node.invalid',
            message: 'arithmetic overflow for trust',
            detail: null,
            retryable: false,
          })
        : prior(name, args),
    )
    mount()
    await screen.findByLabelText('Initial trust')
    fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Continue' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Arithmetic overflow for trust.')
    expect(screen.getByText('I have proof.')).toBeInTheDocument()
  })
})

it('captures the full action tape and frozen command signatures when saving a playthrough', async () => {
  const previous = h.invoke.getMockImplementation()!
  h.invoke.mockImplementation(async (command: string, args: Record<string, unknown>) => {
    if (command === 'narrative_scenario_save')
      return {
        id: 'saved',
        name: args.name,
        scenario: JSON.parse(args.source as string),
        stamp: null,
      }
    return previous(command, args)
  })
  mount()
  await screen.findByLabelText('Initial trust')
  fireEvent.change(screen.getByLabelText('Command signatures (JSON)'), {
    target: { value: '{"award_badge":["bool"]}' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Start preview' }))
  await screen.findByText('I have proof.')
  fireEvent.change(screen.getByLabelText('Command signatures (JSON)'), { target: { value: '{}' } })
  fireEvent.click(screen.getByRole('button', { name: 'Save snapshot' }))
  fireEvent.click(screen.getByRole('button', { name: 'Continue' }))
  fireEvent.click(await screen.findByRole('button', { name: 'Show proof' }))
  fireEvent.click(await screen.findByRole('button', { name: 'Acknowledge command' }))
  await screen.findByText('Scene ended: Supported')
  fireEvent.click(screen.getByRole('button', { name: 'Restore snapshot' }))
  await screen.findByText('I have proof.')
  fireEvent.click(screen.getByText('Saved scenarios'))
  fireEvent.change(screen.getByLabelText('Scenario name'), {
    target: { value: 'Council regression' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Save scenario' }))
  await screen.findByText('Saved “Council regression” with 6 asserted steps.')
  const request = h.invoke.mock.calls.find(([command]) => command === 'narrative_scenario_save')![1]
  const scenario = JSON.parse(request.source)
  expect(scenario.commands).toEqual({ award_badge: ['bool'] })
  expect(
    scenario.steps.map((step: { action: { kind: string } | null }) => step.action?.kind ?? null),
  ).toEqual([
    null,
    'save_checkpoint',
    'advance',
    'choose',
    'complete_command',
    'restore_checkpoint',
  ])
  expect(request.source).not.toContain('I have proof.')
  expect(request.source).not.toContain('token')
})
