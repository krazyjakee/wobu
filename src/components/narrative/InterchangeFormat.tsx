export function InterchangeFormat({
  label,
  csv,
  onChange,
}: {
  label: string
  csv: boolean
  onChange: (csv: boolean) => void
}) {
  return (
    <label>
      {label}{' '}
      <select
        value={csv ? 'csv' : 'json'}
        onChange={(event) => onChange(event.target.value === 'csv')}
      >
        <option value="csv">CSV</option>
        <option value="json">JSON</option>
      </select>
    </label>
  )
}
