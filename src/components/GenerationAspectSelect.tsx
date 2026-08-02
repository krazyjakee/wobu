export function GenerationAspectSelect({
  label,
  value,
  choices,
  onChange,
}: {
  label: string
  value: string
  choices: string[]
  onChange: (value: string) => void
}) {
  return (
    <select
      aria-label={label}
      value={value}
      onChange={(event) => onChange(event.target.value)}
      disabled={!choices.length}
    >
      {choices.map((choice) => (
        <option key={choice} value={choice}>
          {choice}
        </option>
      ))}
    </select>
  )
}
