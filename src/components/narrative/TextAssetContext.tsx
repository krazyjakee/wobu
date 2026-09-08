import type { TextAsset, SourceLink } from '../../lib/api/narrativeText'
import { useNarrativeState, useNodes, useScenes } from '../../lib/queries'
import { useNarrativeWorld } from '../../lib/queries/narrativeWorld'
import { TypedCondition } from './TypedCondition'
import { templateOf } from './textLibraryModel'

export function TextAssetContext({
  asset,
  disabled,
  onChange,
}: {
  asset: TextAsset
  disabled: boolean
  onChange: (asset: TextAsset) => void
}) {
  const schema = useNarrativeState()
  const nodes = useNodes(true)
  const world = useNarrativeWorld()
  const scenes = useScenes()
  const characters = (nodes.data ?? []).filter((node) => node.kind === 'character')
  const options: { key: string; label: string; source: SourceLink }[] = [
    ...(world.data?.document.facts ?? []).map((record) => ({
      key: `fact:${record.id}`,
      label: `Fact: ${record.name}`,
      source: { fact: record.id },
    })),
    ...(world.data?.document.events ?? []).map((record) => ({
      key: `event:${record.id}`,
      label: `Event: ${record.name}`,
      source: { event: record.id },
    })),
    ...(world.data?.document.quests ?? []).map((record) => ({
      key: `quest:${record.id}`,
      label: `Quest: ${record.name}`,
      source: { quest: record.id },
    })),
    ...characters.map((record) => ({
      key: `character:${record.id}`,
      label: `Character: ${record.name}`,
      source: { character: record.id },
    })),
    ...(scenes.data?.scenes ?? []).map((record) => ({
      key: `scene:${record.id}`,
      label: `Scene: ${record.name}`,
      source: { scene: record.id },
    })),
  ]
  const sourceKey = (source: SourceLink) =>
    Object.entries(source)
      .map(([kind, id]) => `${kind}:${id}`)
      .join('')
  return (
    <fieldset disabled={disabled} className="ntl-context-fields">
      <legend>Trigger and context</legend>
      <TypedCondition
        label="Trigger condition"
        value={asset.trigger.when}
        variables={schema.data?.document.variables ?? []}
        onChange={(when) => onChange({ ...asset, trigger: { ...asset.trigger, when } })}
      />
      {templateOf(asset.kind).cast && (
        <fieldset>
          <legend>Cast</legend>
          {characters.map((character) => (
            <label key={character.id} className="ntl-check">
              <input
                type="checkbox"
                checked={
                  asset.participants?.some((participant) => participant.entity === character.id) ??
                  false
                }
                onChange={(event) =>
                  onChange({
                    ...asset,
                    participants: event.target.checked
                      ? [...(asset.participants ?? []), { entity: character.id }]
                      : asset.participants?.filter(
                          (participant) => participant.entity !== character.id,
                        ),
                  })
                }
              />
              {character.name}
            </label>
          ))}
          {!characters.length && <p>Create characters in World to assign entity speakers.</p>}
        </fieldset>
      )}
      <label>
        Relevant source links
        <select
          value=""
          onChange={(event) => {
            const option = options.find((option) => option.key === event.target.value)
            if (option && !asset.sources?.some((source) => sourceKey(source) === option.key))
              onChange({ ...asset, sources: [...(asset.sources ?? []), option.source] })
          }}
        >
          <option value="">Add a source…</option>
          {options.map((option) => (
            <option key={option.key} value={option.key}>
              {option.label}
            </option>
          ))}
        </select>
      </label>
      <ul>
        {asset.sources?.map((source) => (
          <li key={sourceKey(source)}>
            {options.find((option) => option.key === sourceKey(source))?.label ??
              `${sourceKey(source)} (unresolved)`}{' '}
            <button
              className="btn"
              onClick={() =>
                onChange({
                  ...asset,
                  sources: asset.sources?.filter((one) => sourceKey(one) !== sourceKey(source)),
                })
              }
            >
              Remove source
            </button>
          </li>
        ))}
      </ul>
      <label>
        Must convey
        <textarea
          rows={2}
          value={asset.must_convey?.join('\n') ?? ''}
          onChange={(event) => onChange({ ...asset, must_convey: event.target.value.split('\n') })}
        />
      </label>
      <label>
        Must not reveal
        <textarea
          rows={2}
          value={asset.must_not_reveal?.join('\n') ?? ''}
          onChange={(event) =>
            onChange({ ...asset, must_not_reveal: event.target.value.split('\n') })
          }
        />
      </label>
    </fieldset>
  )
}
