import type { NarrativeFilter } from '../../store/ui'

/** Shared status vocabulary and explanations for unavailable Narrative actions. */

/**
 * A row's condition, said in words.
 *
 * The three the writer filters by are the store's `NarrativeFilter` values, so
 * a filter and the badge it matches are one vocabulary rather than two lists
 * that have to be kept in step.
 */
export type NarrativeStatus = NarrativeFilter | 'locked' | 'ready'

/** Every status carries a label and a glyph: colour alone is not a status. */
export const NARRATIVE_STATUS: Record<NarrativeStatus, { label: string; icon: string }> = {
  needsText: { label: 'Needs text', icon: 'spark' },
  needsReview: { label: 'Needs review', icon: 'clock' },
  outOfDate: { label: 'Out of date', icon: 'refresh' },
  locked: { label: 'Locked', icon: 'lock' },
  ready: { label: 'Ready', icon: 'check' },
}

/**
 * Why each control is refused today.
 *
 * Written as what is missing rather than as "coming soon": a reader deciding
 * whether they have hit a bug is owed the actual reason, and none of these is
 * a failure they can retry.
 */
export const NARRATIVE_UNAVAILABLE = {
  quests:
    'Quest membership is available in the Scene library. Grouping the arc canvas by those memberships is not available yet (#187).',
  flow: 'No scene is selected, so there is nothing to draw. Choose one in the Library, or open one from the arc.',
  witness:
    'Opening a witness needs generated reachability scenarios and a Flow overlay (#171, #188). Preview can play a chosen starting state; these source diagnostics do not establish reachability.',
  affectedScope:
    'Highlighting affected beats needs the dependency tracker and build planner (#168, #169).',
  review:
    'The generation proposal comparison and review queue are not available yet (#165, #166). Handwritten text can be edited in Script.',
  diagnostics:
    'Checks cover source errors and missing text. Branch reachability has not been checked.',
} as const
