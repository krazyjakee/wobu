import type { Destination, Scene } from '../../lib/api'

export function DestinationPicker({
  fieldId,
  label,
  value,
  scene,
  scenes,
  onChange,
}: {
  fieldId: string
  label: string
  value: Destination
  scene: Scene
  scenes: { id: string; name: string }[]
  onChange: (value: Destination) => void
}) {
  const selected =
    'beat' in value
      ? `beat:${value.beat}`
      : 'scene' in value
        ? `scene:${value.scene}`
        : 'end' in value
          ? 'end'
          : 'unresolved'
  const options = [
    { id: 'unresolved', title: 'Unresolved — choose a destination' },
    { id: 'end', title: 'End scene' },
    ...(scene.beats ?? []).map((beat) => ({ id: `beat:${beat.id}`, title: `Beat: ${beat.title}` })),
    ...scenes.map((one) => ({ id: `scene:${one.id}`, title: `Scene: ${one.name}` })),
  ]
  return (
    <>
      <label>
        {label}
        <select
          data-narrative-field={fieldId}
          value={selected}
          onChange={(e) => {
            const key = e.target.value
            onChange(
              key === 'unresolved'
                ? { unresolved: {} }
                : key === 'end'
                  ? { end: {} }
                  : key.startsWith('beat:')
                    ? { beat: key.slice(5) }
                    : { scene: key.slice(6) },
            )
          }}
        >
          {!options.some((one) => one.id === selected) && (
            <option value={selected}>Missing destination: {selected}</option>
          )}
          {options.map((one) => (
            <option key={one.id} value={one.id}>
              {one.title}
            </option>
          ))}
        </select>
      </label>
      {'end' in value && (
        <label>
          {label} ending label
          <input
            value={value.end.label ?? ''}
            onChange={(event) => onChange({ end: { label: event.target.value } })}
          />
        </label>
      )}
    </>
  )
}
