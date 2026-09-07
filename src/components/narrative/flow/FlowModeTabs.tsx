/**
 * Canvas, or the outline list — the choice both Flow levels offer.
 *
 * One component rather than one per level, because #151 requires the outline to
 * be a *full* alternative to the canvas rather than a reduced one, and the
 * first way that promise rots is two switches that drift: one gains a third
 * mode, one loses a label, and the two levels stop meaning the same thing by
 * the same word.
 *
 * A `role="group"` of pressed buttons rather than a `tablist`. The workspace's
 * own tab strip up in `NarrativeCentre` is a real `tablist`, and having two
 * nested ones would make arrow keys ambiguous — the outer strip switches views
 * of a scene, this switches how one view draws itself.
 */

const FLOW_MODES = [
  { id: 'canvas', label: 'Canvas' },
  { id: 'outline', label: 'Outline list' },
] as const

export type FlowMode = (typeof FLOW_MODES)[number]['id']

export function FlowModeTabs({
  mode,
  onMode,
  label,
}: {
  mode: FlowMode
  onMode: (mode: FlowMode) => void
  /** Which level's switch this is, for a reader with two of them in earshot. */
  label: string
}) {
  return (
    <div className="tabs" role="group" aria-label={label}>
      {FLOW_MODES.map((option) => (
        <button
          key={option.id}
          type="button"
          className={mode === option.id ? 'tab is-active' : 'tab'}
          aria-pressed={mode === option.id}
          onClick={() => onMode(option.id)}
        >
          {option.label}
        </button>
      ))}
    </div>
  )
}
