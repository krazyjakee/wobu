import { Icon } from '../../Icon'
import { TipButton } from '../../Tooltip'
import { FLOW_DIAGNOSTIC_CATEGORIES } from './badges'
import { useFlowLevel } from './flowStore'
import { NARRATIVE_STATUS } from '../narrativeModel'

/**
 * The badge legend, and the two filters that govern it.
 *
 * ── the guarantee this component exists to keep ──────────────────────────────
 *
 * **A release-blocking error cannot be hidden.** Not "the UI does not offer a
 * way to hide one" — there is no state that could. `badgeShown` returns true
 * for an error before it reads the filter at all, so the Errors chip below is
 * an always-on marker that explains itself rather than a control, and the
 * category chips are documented as governing warnings only. #187 proves the
 * same property for the node filters from the other end, with `blocking` on the
 * graph node; this is the badge half of it.
 *
 * ── colour-independent, per #151 ─────────────────────────────────────────────
 *
 * Every chip and every legend row carries a glyph and a word. Somebody who
 * cannot tell the two tints apart can still read which category a badge is in
 * and whether it stops a release, and the legend says what lands in each.
 *
 * ── and it writes nothing ────────────────────────────────────────────────────
 *
 * The switches live on the canvas's own zustand store, which is memory for the
 * life of the pane. There is no path from here to a project file, to the layout
 * sidecar or to the query cache, which is #189's "toggling badges never
 * modifies project files" made structural rather than remembered.
 */
export function FlowBadgeFilters() {
  const badges = useFlowLevel((s) => s.badges)
  const setWarnings = useFlowLevel((s) => s.setBadgeWarnings)
  const toggleCategory = useFlowLevel((s) => s.toggleBadgeCategory)

  return (
    <details className="nrt-legend">
      <summary>Badges</summary>
      <div className="nrt-legend-body">
        <div className="nrt-filter-row" role="group" aria-label="Badge severity">
          {/* `aria-disabled` rather than `disabled`, as everywhere else in this
              workspace: a control that cannot be focused cannot say why it is
              refusing, and "why can't I turn errors off" is the question this
              one exists to answer. */}
          <TipButton
            className="chip is-on"
            aria-pressed="true"
            disabledReason="Errors block a release, so no filter can hide them. This one is on, and there is no state in which it is not."
            tip="Problems that stop the story working"
          >
            <Icon name="x" size="sm" />
            Errors
          </TipButton>
          <button
            type="button"
            className={badges.warnings ? 'chip is-on' : 'chip'}
            aria-pressed={badges.warnings}
            onClick={() => setWarnings(!badges.warnings)}
          >
            <Icon name="clock" size="sm" />
            Warnings
          </button>
        </div>

        <div className="nrt-filter-row" role="group" aria-label="Badge categories">
          {FLOW_DIAGNOSTIC_CATEGORIES.map((category) => (
            <button
              key={category.id}
              type="button"
              className={badges.categories[category.id] ? 'chip is-on' : 'chip'}
              aria-pressed={badges.categories[category.id]}
              onClick={() => toggleCategory(category.id)}
            >
              <Icon name={category.icon} size="sm" />
              {category.label}
            </button>
          ))}
        </div>

        <ul className="nrt-legend-rows" aria-label="What each badge means">
          {FLOW_DIAGNOSTIC_CATEGORIES.map((category) => (
            <li key={category.id}>
              <span className="nrt-badge">
                <Icon name={category.icon} size="sm" />
                {category.label}
              </span>
              <span>{category.about}</span>
            </li>
          ))}
          {/* The lifecycle counts are not diagnostics and are not filtered:
              they are the beat's own outstanding work, three independent
              dimensions that a single status would collapse. */}
          {(['needsText', 'needsReview', 'outOfDate', 'locked'] as const).map((status) => (
            <li key={status}>
              <span className="nrt-badge">
                <Icon name={NARRATIVE_STATUS[status].icon} size="sm" />
                {NARRATIVE_STATUS[status].label}
              </span>
              <span>{LIFECYCLE_ABOUT[status]}</span>
            </li>
          ))}
        </ul>
        <p className="nrt-note">
          Category chips govern warnings. Needs text, needs review, out of date and locked are
          counted separately on every beat, because a line can be more than one of them at once.
        </p>
      </div>
    </details>
  )
}

/** Said in the legend, so the three dimensions are not merely three words. */
const LIFECYCLE_ABOUT = {
  needsText: 'A dialogue slot with no wording in it yet.',
  needsReview: 'Wording nobody has approved at this revision.',
  outOfDate: 'Wording whose context has moved since it was written — locked or not.',
  locked: 'Wording no generation job may touch. Still counted as out of date when it is.',
} as const
