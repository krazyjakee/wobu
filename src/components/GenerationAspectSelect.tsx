import { useMemo } from 'react'
import { Combobox } from './Combobox'

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
  /*
   * `sort="none"`: the choices arrive from the provider negotiation in a
   * meaningful sequence — squarest first, then wider and taller — and sorting
   * "16:9" next to "1:1" alphabetically would scramble a list the user reads as
   * a scale rather than as names.
   */
  const options = useMemo(
    () => choices.map((choice) => ({ value: choice, label: choice })),
    [choices],
  )
  return (
    <Combobox
      label={label}
      value={value}
      options={options}
      onChange={onChange}
      disabledReason={
        choices.length
          ? null
          : 'The image backend has not said which aspect ratios it accepts. Check the backend is connected in Settings.'
      }
      placeholder="No aspects offered"
    />
  )
}
