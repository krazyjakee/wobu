import { describe, expect, it } from 'vitest'
import type { PreviewFrame } from '../../lib/api/narrativePreview'
import { appendTape, assertFrame, MAX_SCENARIO_STEPS, type ScenarioTape } from './scenarioTape'
const frame: PreviewFrame = {
  snapshot: {},
  state: { trust: 50 },
  current: {
    line: {
      scene: 'scene',
      beat: 'beat',
      slot: 'slot',
      variant: 'variant',
      speaker: 'player',
      text: 'Temporary wording',
      revision: 'temporary-revision',
    },
  },
  trace: { committed: true, error: null, omitted: 0, records: [] },
  branch: [],
  build: 'build-1',
}
describe('scenario capture', () => {
  it('records stable identities and state without prose, revision or command tokens', () => {
    const step = assertFrame(frame, null)
    expect(step.expect).toEqual({
      boundary: { kind: 'line', scene: 'scene', beat: 'beat', slot: 'slot', variant: 'variant' },
      state: { trust: 50 },
      error: null,
    })
    const command = assertFrame(
      {
        ...frame,
        current: { game_command: { token: 'ephemeral', name: 'file_record', args: [true] } },
      },
      { kind: 'complete_command', result: 'cancelled' },
    )
    expect(JSON.stringify(command)).not.toContain('ephemeral')
    expect(command.expect.error).toBe('command_cancelled')
  })
  it('keeps the first recorded step when the explicit tape cap is reached and refuses further capture', () => {
    const first = assertFrame(frame, null)
    const tape: ScenarioTape = {
      incomplete: false,
      scenario: {
        version: 1,
        scene: 'scene',
        initial_state: {},
        seed: 0,
        commands: {},
        steps: [
          first,
          ...Array.from({ length: MAX_SCENARIO_STEPS - 1 }, () =>
            assertFrame(frame, { kind: 'advance' }),
          ),
        ],
      },
    }
    const next = appendTape(tape, frame, { kind: 'save_checkpoint' })!
    expect(next.incomplete).toBe(true)
    expect(next.scenario.steps).toHaveLength(MAX_SCENARIO_STEPS)
    expect(next.scenario.steps[0]).toEqual(first)
  })
})
