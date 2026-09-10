import type { ReactNode } from 'react'
import type { VariableDecl } from '../../lib/api'
import type { QuestStage, WorldDocument } from '../../lib/api/narrativeWorld'
import {
  renameStage,
  setStageObjective,
  stageName,
  stageObjective,
} from '../../lib/api/narrativeWorld'
import type { WorldItem } from './worldModel'
import { parseSafeInteger } from './integerInput'

/** Stage names as picker options, whichever shape the file wrote them in. */
const stageOptions = (stages: QuestStage[]) =>
  stages.map((stage) => ({ id: stageName(stage), name: stageName(stage) }))

import { TypedCondition as ConditionEditor } from './TypedCondition'

export interface NamedOption {
  id: string
  name: string
}
export function RecordPicker({
  label,
  value,
  options,
  onChange,
}: {
  label: string
  value: string
  options: NamedOption[]
  onChange: (value: string) => void
}) {
  return (
    <label>
      {label}
      <select value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">Choose…</option>
        {options.map((one) => (
          <option key={one.id} value={one.id}>
            {one.name}
          </option>
        ))}
        {value && !options.some((one) => one.id === value) && (
          <option value={value}>Missing record: {value}</option>
        )}
      </select>
    </label>
  )
}
export function MultiplePicker({
  label,
  values,
  options,
  onChange,
}: {
  label: string
  values: string[]
  options: NamedOption[]
  onChange: (values: string[]) => void
}) {
  const present = new Set(options.map((one) => one.id))
  const all = [
    ...options,
    ...values.filter((id) => !present.has(id)).map((id) => ({ id, name: `Missing record: ${id}` })),
  ]
  return (
    <label>
      {label}
      <select
        multiple
        value={values}
        onChange={(e) => onChange(Array.from(e.target.selectedOptions, (option) => option.value))}
      >
        {all.map((one) => (
          <option key={one.id} value={one.id}>
            {one.name}
          </option>
        ))}
      </select>
    </label>
  )
}
function Lines({
  label,
  values,
  onChange,
}: {
  label: string
  values: string[]
  onChange: (values: string[]) => void
}) {
  return (
    <label>
      {label}
      <textarea
        value={values.join('\n')}
        onChange={(e) => onChange(e.target.value ? e.target.value.split('\n') : [])}
      />
    </label>
  )
}
export function WorldFields({
  item,
  document,
  entities,
  characters,
  scenes,
  variables,
  onChange,
}: {
  item: WorldItem
  document: WorldDocument
  entities: NamedOption[]
  characters: NamedOption[]
  scenes: NamedOption[]
  variables: VariableDecl[]
  onChange: (item: WorldItem) => void
}) {
  let fields: ReactNode = null
  if ('assertion' in item) {
    fields = (
      <>
        <label>
          Canonical assertion
          <textarea
            value={item.assertion}
            onChange={(e) => onChange({ ...item, assertion: e.target.value })}
          />
        </label>
        <Lines
          label="Sources (one per line)"
          values={item.sources}
          onChange={(sources) => onChange({ ...item, sources })}
        />
      </>
    )
  } else if ('belief' in item) {
    const kind =
      typeof item.provenance === 'string' ? item.provenance : Object.keys(item.provenance)[0]
    fields = (
      <>
        <RecordPicker
          label="Character"
          value={item.character}
          options={characters}
          onChange={(character) => onChange({ ...item, character })}
        />
        <RecordPicker
          label="Fact"
          value={item.fact}
          options={document.facts}
          onChange={(fact) => onChange({ ...item, fact })}
        />
        <label>
          Belief
          <select
            value={item.belief}
            onChange={(e) => onChange({ ...item, belief: e.target.value as typeof item.belief })}
          >
            <option value="true">Believes the assertion</option>
            <option value="false">Believes the assertion is false</option>
            <option value="unknown">Does not know</option>
          </select>
        </label>
        <p className="nrt-note">
          Beliefs describe this character. They do not change the canonical fact.
        </p>
        <label>
          How they learned it
          <select
            value={kind}
            onChange={(e) =>
              onChange({
                ...item,
                provenance:
                  e.target.value === 'witnessed'
                    ? 'witnessed'
                    : e.target.value === 'told'
                      ? { told: { by: '' } }
                      : e.target.value === 'rumour'
                        ? { rumour: { source: '' } }
                        : { inferred: { reason: '' } },
              })
            }
          >
            <option value="witnessed">Witnessed</option>
            <option value="told">Told by someone</option>
            <option value="rumour">Heard a rumour</option>
            <option value="inferred">Inferred</option>
          </select>
        </label>
        {typeof item.provenance === 'object' && 'told' in item.provenance && (
          <RecordPicker
            label="Told by"
            value={item.provenance.told.by}
            options={characters}
            onChange={(by) => onChange({ ...item, provenance: { told: { by } } })}
          />
        )}
        {typeof item.provenance === 'object' && 'rumour' in item.provenance && (
          <label>
            Rumour source
            <input
              value={item.provenance.rumour.source}
              onChange={(e) =>
                onChange({ ...item, provenance: { rumour: { source: e.target.value } } })
              }
            />
          </label>
        )}
        {typeof item.provenance === 'object' && 'inferred' in item.provenance && (
          <label>
            Reasoning
            <textarea
              value={item.provenance.inferred.reason}
              onChange={(e) =>
                onChange({ ...item, provenance: { inferred: { reason: e.target.value } } })
              }
            />
          </label>
        )}
      </>
    )
  } else if ('from' in item) {
    fields = (
      <>
        <RecordPicker
          label="From"
          value={item.from}
          options={characters}
          onChange={(from) => onChange({ ...item, from })}
        />
        <RecordPicker
          label="Towards"
          value={item.to}
          options={characters}
          onChange={(to) => onChange({ ...item, to })}
        />
        <label>
          Relationship kind
          <input value={item.kind} onChange={(e) => onChange({ ...item, kind: e.target.value })} />
        </label>
        <label>
          Value type
          <select
            value={typeof item.value}
            onChange={(e) =>
              onChange({
                ...item,
                value: e.target.value === 'boolean' ? false : e.target.value === 'number' ? 0 : '',
              })
            }
          >
            <option value="number">Integer</option>
            <option value="boolean">Boolean</option>
            <option value="string">Named value</option>
          </select>
        </label>
        <label>
          Relationship value
          {typeof item.value === 'boolean' ? (
            <select
              value={String(item.value)}
              onChange={(e) => onChange({ ...item, value: e.target.value === 'true' })}
            >
              <option value="true">True</option>
              <option value="false">False</option>
            </select>
          ) : (
            <input
              type={typeof item.value === 'number' ? 'number' : 'text'}
              value={item.value}
              onChange={(e) => {
                const value =
                  typeof item.value === 'number' ? parseSafeInteger(e.target.value) : e.target.value
                if (value !== undefined) onChange({ ...item, value })
              }}
            />
          )}
        </label>
      </>
    )
  } else if ('stages' in item) {
    fields = (
      <>
        <label>
          Summary
          <textarea
            value={item.summary}
            onChange={(e) => onChange({ ...item, summary: e.target.value })}
          />
        </label>
        <Lines
          label="Quest stages (one name per line)"
          values={item.stages.map(stageName)}
          // Renaming or reordering keeps each stage's objective, so editing this
          // list does not silently drop wording somebody wrote. A line that
          // still names a stage keeps that stage wherever it moved to; a line
          // that names none was renamed in place, so it inherits the stage that
          // held its position — unless that stage is still listed elsewhere
          // under its own name, which makes this line a new one.
          onChange={(names) =>
            onChange({
              ...item,
              stages: names.map((name, index) => {
                const existing = item.stages.find((one) => stageName(one) === name)
                if (existing) return existing
                const held = item.stages[index]
                if (!held || names.includes(stageName(held))) return name
                return renameStage(held, name)
              }),
            })
          }
        />
        {item.stages.map((stage, index) => (
          <label key={stageName(stage)}>
            {`Objective for ${stageName(stage)}`}
            <textarea
              data-narrative-field={`quest:objective:${stageName(stage)}`}
              value={stageObjective(stage)?.text.body ?? ''}
              placeholder="What the player should do while the quest is at this stage."
              onChange={(e) =>
                onChange({
                  ...item,
                  stages: item.stages.map((one, i) =>
                    i === index ? setStageObjective(one, e.target.value) : one,
                  ),
                })
              }
            />
          </label>
        ))}
        <RecordPicker
          label="Initial stage"
          value={item.initial}
          options={stageOptions(item.stages)}
          onChange={(initial) => onChange({ ...item, initial })}
        />
        <MultiplePicker
          label="Scenes in this quest"
          values={item.scene_ids}
          options={scenes}
          onChange={(scene_ids) => onChange({ ...item, scene_ids })}
        />
        {item.transitions.map((transition, index) => (
          <fieldset key={index}>
            <legend>Transition {index + 1}</legend>
            <RecordPicker
              label={`Transition ${index + 1} from`}
              value={transition.from}
              options={stageOptions(item.stages)}
              onChange={(from) =>
                onChange({
                  ...item,
                  transitions: item.transitions.map((one, i) =>
                    i === index ? { ...one, from } : one,
                  ),
                })
              }
            />
            <RecordPicker
              label={`Transition ${index + 1} to`}
              value={transition.to}
              options={stageOptions(item.stages)}
              onChange={(to) =>
                onChange({
                  ...item,
                  transitions: item.transitions.map((one, i) =>
                    i === index ? { ...one, to } : one,
                  ),
                })
              }
            />
            <ConditionEditor
              label={`Transition ${index + 1} condition`}
              value={transition.when}
              variables={variables}
              onChange={(when) =>
                onChange({
                  ...item,
                  transitions: item.transitions.map((one, i) =>
                    i === index ? { ...one, when: when ?? 'always' } : one,
                  ),
                })
              }
            />
            <button
              type="button"
              className="btn"
              onClick={() =>
                onChange({ ...item, transitions: item.transitions.filter((_, i) => i !== index) })
              }
            >
              Remove transition {index + 1}
            </button>
          </fieldset>
        ))}
        <button
          type="button"
          className="btn"
          onClick={() =>
            onChange({
              ...item,
              transitions: [
                ...item.transitions,
                {
                  from: item.initial,
                  to: item.stages.at(-1) ? stageName(item.stages.at(-1)!) : item.initial,
                  when: 'always',
                },
              ],
            })
          }
        >
          Add transition
        </button>
      </>
    )
  } else if ('until' in item) {
    fields = (
      <>
        <RecordPicker
          label="Restricted fact"
          value={item.fact}
          options={document.facts}
          onChange={(fact) => onChange({ ...item, fact })}
        />
        <MultiplePicker
          label="Restricted characters"
          values={item.characters}
          options={characters}
          onChange={(characters) => onChange({ ...item, characters })}
        />
        <p className="nrt-note">
          Selected characters must not reveal this fact until the condition is true. An empty
          selection applies to everyone. Never keeps it restricted.
        </p>
        <ConditionEditor
          label="May reveal when"
          value={item.until}
          variables={variables}
          onChange={(until) => onChange({ ...item, until: until ?? 'never' })}
        />
      </>
    )
  } else if ('fact_ids' in item) {
    fields = (
      <>
        <label>
          Summary
          <textarea
            value={item.summary}
            onChange={(e) => onChange({ ...item, summary: e.target.value })}
          />
        </label>
        <MultiplePicker
          label="Facts established by this event"
          values={item.fact_ids}
          options={document.facts}
          onChange={(fact_ids) => onChange({ ...item, fact_ids })}
        />
      </>
    )
  }
  return (
    <>
      <label>
        Record name
        <input value={item.name} onChange={(e) => onChange({ ...item, name: e.target.value })} />
      </label>
      {fields}
      {('assertion' in item || 'fact_ids' in item) && (
        <MultiplePicker
          label="Related world entities"
          values={item.entity_ids ?? []}
          options={entities}
          onChange={(entity_ids) => onChange({ ...item, entity_ids })}
        />
      )}
      {'when' in item && (
        <ConditionEditor
          label="Applies when"
          value={item.when}
          variables={variables}
          onChange={(when) => onChange({ ...item, when: when ?? 'always' })}
        />
      )}
    </>
  )
}
