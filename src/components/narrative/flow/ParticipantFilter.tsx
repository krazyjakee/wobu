import { useUI, type NarrativeFilter } from '../../../store/ui'
import { NARRATIVE_STATUS } from '../narrativeModel'
import { Icon } from '../../Icon'
import { useMemo } from 'react'
import { useFlowLevel } from './flowStore'
import type { FlowLevel } from './model'

export function ParticipantFilter({ scene }: { scene: FlowLevel }) {
  const participant = useFlowLevel((s) => s.participant)
  const setParticipant = useFlowLevel((s) => s.setParticipant)
  const people = useMemo(() => {
    const names = new Map<string, string>()
    for (const element of scene.elements) {
      // Beats have participants and so do whole scenes, which is what makes
      // "show me only Mira's thread" mean the same thing at both levels.
      if (element.kind === 'beat' || element.kind === 'scene') {
        for (const name of element.participants)
          names.set(
            name,
            element.kind === 'scene' ? (element.participantLabels?.[name] ?? name) : name,
          )
      }
    }
    return [...names].sort((a, b) => a[1].localeCompare(b[1]))
  }, [scene])

  return (
    <label className="nrt-bar-field">
      <span>Participant</span>
      <select
        value={participant ?? ''}
        onChange={(event) => setParticipant(event.target.value || null)}
      >
        <option value="">Anyone</option>
        {people.map(([id, name]) => (
          <option key={id} value={id}>
            {name}
          </option>
        ))}
      </select>
    </label>
  )
}

export function FlowStatusFilters() {
  const filters = useUI((state) => state.narrativeFilters)
  const toggle = useUI((state) => state.toggleNarrativeFilter)
  return (
    <div role="group" aria-label="Flow text status filters" className="nrt-filter-row">
      {(['needsText', 'needsReview', 'outOfDate'] as NarrativeFilter[]).map((key) => (
        <button
          type="button"
          className={filters[key] ? 'chip is-on' : 'chip'}
          key={key}
          aria-pressed={filters[key]}
          onClick={() => toggle(key)}
        >
          <Icon name={NARRATIVE_STATUS[key].icon} size="sm" />
          {NARRATIVE_STATUS[key].label}
        </button>
      ))}
    </div>
  )
}
