import { fireEvent, screen, within } from '@testing-library/react'

/**
 * Driving `Combobox` from a test, the way a keyboard user drives it.
 *
 * Shared rather than repeated in each suite because the popup is a portal: it
 * is not inside the panel the test rendered, so `within(panel)` finds the field
 * and never the rows. Getting that wrong reads as "the option is missing",
 * which is the same symptom as a real filtering bug — so the query belongs in
 * one place with a name on it.
 *
 * Not a `.test.ts` file, so Vitest does not collect it as a suite.
 */

const box = (name: string | RegExp, container?: HTMLElement) =>
  (container ? within(container) : screen).getByRole('combobox', { name })

/** Open the named picker. Returns its field, so callers can assert on it. */
export function openCombobox(name: string | RegExp, container?: HTMLElement): HTMLElement {
  const field = box(name, container)
  fireEvent.keyDown(field, { key: 'ArrowDown' })
  return field
}

/** The row labels the named picker is currently offering, list left open. */
export function comboboxOptions(name: string | RegExp, container?: HTMLElement): string[] {
  openCombobox(name, container)
  return screen.queryAllByRole('option').map((row) => row.textContent ?? '')
}

/** Open the named picker and click one of its rows. */
export function chooseOption(
  name: string | RegExp,
  option: string | RegExp,
  container?: HTMLElement,
) {
  openCombobox(name, container)
  fireEvent.click(screen.getByRole('option', { name: option }))
}

/** Type into the named picker and take the highlighted row with Enter. */
export function filterAndChoose(
  name: string | RegExp,
  query: string,
  container?: HTMLElement,
): HTMLElement {
  const field = box(name, container)
  fireEvent.change(field, { target: { value: query } })
  fireEvent.keyDown(field, { key: 'Enter' })
  return field
}
