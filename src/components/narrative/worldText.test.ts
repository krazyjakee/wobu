import { describe, expect, it, vi, beforeEach } from 'vitest'
import type { WorldDocument } from '../../lib/api/narrativeWorld'
import { setStageObjective, stageName, stageObjective } from '../../lib/api/narrativeWorld'
import { prepareWorldText } from './worldText'

const written = vi.fn()
vi.mock('../../lib/api', () => ({
  narrativeTextWritten: (...args: unknown[]) => written(...args),
}))
vi.mock('./sceneIdentity', () => ({ mintId: () => 'MINTED' }))

function world(stages: WorldDocument['quests']): WorldDocument {
  return { schema_version: 2, quests: stages } as WorldDocument
}
const quest = (stages: unknown[]) =>
  [
    {
      id: 'q1',
      name: 'A shift at the diner',
      summary: '',
      stages,
      initial: 'available',
      transitions: [],
      scene_ids: [],
    },
  ] as WorldDocument['quests']

beforeEach(() => {
  written.mockReset()
  written.mockImplementation((body: string) => ({ revision: `sealed:${body}`, body }))
})

describe('Quest stage objectives', () => {
  it('leaves a stage with no objective as the bare name the file wrote', () => {
    // An existing World file nobody has written an objective for must not be
    // rewritten by being opened and saved.
    const bare = setStageObjective('available', '   ')
    expect(bare).toBe('available')
    expect(stageObjective(bare)).toBeNull()
    expect(stageName(bare)).toBe('available')
  })

  it('seals changed words once and mints an id only for a brand new objective', async () => {
    const before = world(quest(['available']))
    const draft = world(quest([setStageObjective('available', 'Find Rosa at the diner.')]))

    const sealed = await prepareWorldText(before, draft)
    const objective = stageObjective(sealed.quests![0]!.stages[0]!)!
    expect(objective.id).toBe('MINTED')
    expect(objective.text.revision).toBe('sealed:Find Rosa at the diner.')
    expect(written).toHaveBeenCalledTimes(1)
  })

  it('does not reseal wording nobody changed, so a translation keyed to it survives', async () => {
    const existing = {
      name: 'available',
      objective: { id: 'OBJ', text: { revision: 'sealed:Find Rosa.', body: 'Find Rosa.' } },
    }
    const before = world(quest([existing]))
    const unchanged = await prepareWorldText(before, world(quest([existing])))
    expect(written).not.toHaveBeenCalled()
    expect(stageObjective(unchanged.quests![0]!.stages[0]!)!.text.revision).toBe(
      'sealed:Find Rosa.',
    )

    // Editing the words reseals, and keeps the identity the locale row is under.
    const edited = setStageObjective(existing, 'Find Rosa at the diner.')
    const next = await prepareWorldText(before, world(quest([edited])))
    const objective = stageObjective(next.quests![0]!.stages[0]!)!
    expect(objective.id).toBe('OBJ')
    expect(objective.text.revision).toBe('sealed:Find Rosa at the diner.')
  })

  it('removes an objective whose words are cleared rather than storing a blank one', async () => {
    const existing = {
      name: 'available',
      objective: { id: 'OBJ', text: { revision: 'sealed:Find Rosa.', body: 'Find Rosa.' } },
    }
    const cleared = await prepareWorldText(
      world(quest([existing])),
      world(quest([setStageObjective(existing, '')])),
    )
    expect(cleared.quests![0]!.stages[0]).toBe('available')
    expect(written).not.toHaveBeenCalled()
  })
})
