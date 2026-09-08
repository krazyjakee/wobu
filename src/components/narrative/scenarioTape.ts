import type { Scenario, ScenarioAction, ScenarioStep } from '../../lib/api/narrativeScenarios'
import type { PreviewFrame } from '../../lib/api/narrativePreview'

export const MAX_SCENARIO_STEPS = 1000
export interface ScenarioTape {
  scenario: Scenario
  incomplete: boolean
}
/** Wording and ephemeral command tokens never enter a regression assertion. */
export function assertFrame(frame: PreviewFrame, action: ScenarioAction | null): ScenarioStep {
  const current = frame.current
  const boundary: ScenarioStep['expect']['boundary'] =
    'line' in current
      ? {
          kind: 'line',
          scene: current.line.scene,
          beat: current.line.beat,
          slot: current.line.slot,
          variant: current.line.variant,
        }
      : 'choices' in current
        ? {
            kind: 'choices',
            scene: current.choices.scene,
            beat: current.choices.beat,
            ids: current.choices.choices.map((choice) => choice.id),
          }
        : 'game_command' in current
          ? { kind: 'command', name: current.game_command.name, args: current.game_command.args }
          : {
              kind: 'end',
              ...(frame.site?.scene ? { scene: frame.site.scene } : {}),
              ...(frame.site?.beat ? { beat: frame.site.beat } : {}),
            }
  const error =
    action?.kind === 'complete_command'
      ? action.result === 'cancelled'
        ? 'command_cancelled'
        : 'failed' in action.result
          ? 'command_failed'
          : null
      : null
  return { action, expect: { boundary, state: structuredClone(frame.state), error } }
}
export function appendTape(
  tape: ScenarioTape | undefined,
  frame: PreviewFrame,
  action: ScenarioAction,
): ScenarioTape | undefined {
  if (!tape) return undefined
  if (tape.incomplete || tape.scenario.steps.length >= MAX_SCENARIO_STEPS)
    return { ...tape, incomplete: true }
  return {
    ...tape,
    scenario: { ...tape.scenario, steps: [...tape.scenario.steps, assertFrame(frame, action)] },
  }
}
