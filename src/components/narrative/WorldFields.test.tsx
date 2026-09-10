import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { Quest, WorldDocument } from '../../lib/api/narrativeWorld'
import { stageName, stageObjective } from '../../lib/api/narrativeWorld'
import { WorldFields } from './WorldFields'

const document: WorldDocument = {
  schema_version: 1,
  facts: [],
  knowledge: [],
  relationships: [],
  events: [],
  quests: [],
  restrictions: [],
}
const quest: Quest = {
  id: 'lunch_rush',
  name: 'Lunch rush',
  summary: 'Work a shift at the diner.',
  stages: [
    { name: 'available', objective: { id: 'obj-1', text: { revision: 'r1', body: 'Find Rosa.' } } },
    'completed',
  ],
  initial: 'available',
  transitions: [],
  scene_ids: [],
}
function mount(item: Quest = quest) {
  const onChange = vi.fn()
  render(
    <WorldFields
      item={item}
      document={document}
      entities={[]}
      characters={[]}
      scenes={[]}
      variables={[]}
      onChange={onChange}
    />,
  )
  return onChange
}
const stages = (onChange: ReturnType<typeof vi.fn>) => (onChange.mock.calls[0]![0] as Quest).stages

describe('quest stage editing', () => {
  it('keeps the objective when its stage is renamed', () => {
    const onChange = mount()
    fireEvent.change(screen.getByLabelText('Quest stages (one name per line)'), {
      target: { value: 'available_now\ncompleted' },
    })
    const renamed = stages(onChange)[0]!
    const untouched = stages(onChange)[1]!
    expect(stageName(renamed)).toBe('available_now')
    expect(stageObjective(renamed)?.text).toEqual({ revision: 'r1', body: 'Find Rosa.' })
    // A stage nobody renamed stays exactly the bare shape the file used.
    expect(untouched).toBe('completed')
  })

  it('keeps each objective with its stage when the list is reordered', () => {
    const onChange = mount()
    fireEvent.change(screen.getByLabelText('Quest stages (one name per line)'), {
      target: { value: 'completed\navailable' },
    })
    expect(stages(onChange).map(stageName)).toEqual(['completed', 'available'])
    expect(stageObjective(stages(onChange)[1]!)?.text.body).toBe('Find Rosa.')
  })

  it('adds a bare stage rather than copying an existing objective onto it', () => {
    const onChange = mount()
    fireEvent.change(screen.getByLabelText('Quest stages (one name per line)'), {
      target: { value: 'started\navailable\ncompleted' },
    })
    expect(stages(onChange)[0]).toBe('started')
    expect(stageObjective(stages(onChange)[1]!)?.text.body).toBe('Find Rosa.')
  })
})
