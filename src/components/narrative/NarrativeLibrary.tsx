import type { ReactNode } from 'react'
import { useUI, NARRATIVE_FILTERS, type NarrativeFilter } from '../../store/ui'
import { Icon } from '../Icon'
import { TipButton } from '../Tooltip'
import { NarrativeList } from './NarrativeList'
import {
  NARRATIVE_SECTIONS,
  NARRATIVE_STATUS,
  NARRATIVE_UNAVAILABLE,
  type NarrativeListState,
  type NarrativeSectionId,
} from './narrativeModel'

/**
 * The left navigator of the Narrative workspace: find things, and filter to the
 * work that is outstanding.
 *
 * The sections use the workspace store's existing `bands` record rather than a
 * narrative-specific open/closed map, so collapsing everything with the
 * keyboard reaches these headings too and one person's idea of a tidy sidebar
 * stays one mechanism instead of two that drift.
 *
 * Only Scenes writes the shared selection today. The other three sections
 * address elements that are not on the scene → beat → line path — a fact, a
 * bark, a quest — and inventing a second selection for them here would be the
 * beginning of exactly the divergence this workspace exists to avoid.
 */
export function NarrativeLibrary({
  sections,
  readOnly,
  onCreateScene,
  sectionNotes,
}: {
  sections: Record<NarrativeSectionId, NarrativeListState>
  readOnly: boolean
  /**
   * Create the project's first scene, for real.
   *
   * Absent means the caller cannot — a read-only folder, or a caller with no
   * project — and the button refuses itself with the reason rather than
   * disappearing, so a reader learns that scenes exist and why this one is not
   * being written.
   */
  onCreateScene?: () => void
  /** Anything a section has to say beside its rows — files that would not read. */
  sectionNotes?: Partial<Record<NarrativeSectionId, ReactNode>>
}) {
  const bands = useUI((s) => s.bands)
  const setBandOpen = useUI((s) => s.setBandOpen)
  const sceneId = useUI((s) => s.narrative.sceneId)
  const selectNarrative = useUI((s) => s.selectNarrative)
  const filters = useUI((s) => s.narrativeFilters)
  const toggleNarrativeFilter = useUI((s) => s.toggleNarrativeFilter)
  const revealed = useUI((s) => s.narrativeReveal)

  const active = NARRATIVE_FILTERS.filter((f) => filters[f])
  // A scene chosen on the canvas, in Script, or by following a diagnostic
  // scrolls its row into view here. Requests this pane raised itself are
  // dropped: the row a reader just clicked is, by definition, already in view.
  const reveal =
    revealed && revealed.origin !== 'library' && revealed.sceneId !== null
      ? { id: revealed.sceneId, seq: revealed.seq }
      : null

  return (
    <nav className="nrt-library" aria-label="Narrative library">
      {NARRATIVE_SECTIONS.map((section) => {
        const key = bandKey(section.id)
        // Undefined is the default rather than "closed": a writer opening this
        // workspace for the first time should see the sections, not four
        // collapsed headings.
        const open = bands[key] ?? true
        const scenes = section.id === 'scenes'
        const state = sections[section.id]
        // An empty list after filtering is not an empty project, and must not
        // be offered the same way out: "Create first scene" under a filter that
        // hid the nine scenes you have is a lie about what you own.
        const hidden = active.length > 0 && state.kind === 'ready'
        return (
          <section className="nrt-section" key={section.id}>
            <h3>
              <button
                type="button"
                className="nrt-section-head"
                aria-expanded={open}
                onClick={() => setBandOpen(key, !open)}
              >
                <Icon
                  name="chev"
                  size="sm"
                  className={open ? 'nrt-twisty is-open' : 'nrt-twisty'}
                />
                {section.label}
              </button>
            </h3>
            {open && (
              <NarrativeList
                label={section.label}
                state={narrow(state, active)}
                empty={
                  hidden
                    ? `Nothing in ${section.label.toLocaleLowerCase()} is in the states you filtered to.`
                    : section.empty
                }
                readOnly={scenes && readOnly}
                selectedId={scenes ? sceneId : null}
                onActivate={
                  scenes ? (id) => selectNarrative({ sceneId: id }, 'library') : undefined
                }
                emptyActions={
                  scenes && !hidden ? (
                    <FirstSceneActions readOnly={readOnly} onCreate={onCreateScene} />
                  ) : undefined
                }
                reveal={scenes ? reveal : null}
              />
            )}
            {open && sectionNotes?.[section.id]}
          </section>
        )
      })}

      <section className="nrt-section nrt-filters">
        <h3>Filters</h3>
        <div className="nrt-filter-row">
          {NARRATIVE_FILTERS.map((filter) => (
            <button
              key={filter}
              type="button"
              className={filters[filter] ? 'chip is-on' : 'chip'}
              aria-pressed={filters[filter]}
              onClick={() => toggleNarrativeFilter(filter)}
            >
              <Icon name={NARRATIVE_STATUS[filter].icon} size="sm" />
              {NARRATIVE_STATUS[filter].label}
            </button>
          ))}
        </div>
        {active.length > 0 && (
          <p className="nrt-note">
            Showing only rows in {active.length === 1 ? 'that state' : 'those states'}. Counts
            appear here once a project can hold narrative source.
          </p>
        )}
      </section>
    </nav>
  )
}

/**
 * The two ways out of an empty project: one that works, and one that says why
 * it does not.
 *
 * "Create first scene" writes a real scene file through `useCreateScene`, which
 * is undoable and guarded like every other narrative write. The Ashfall example
 * still is not possible — unpacking it needs a command that writes a whole
 * project at once — so it refuses itself with that reason rather than being
 * quietly dropped.
 */
function FirstSceneActions({ readOnly, onCreate }: { readOnly: boolean; onCreate?: () => void }) {
  const refusal = readOnly
    ? 'This project folder is read-only, so nothing can be written into it.'
    : null
  return (
    <>
      {onCreate && !refusal ? (
        <button type="button" className="btn btn-primary" onClick={onCreate}>
          Create first scene
        </button>
      ) : (
        <TipButton
          className="btn btn-primary"
          disabledReason={refusal ?? NARRATIVE_UNAVAILABLE.create}
          tip="Write the first scene of this project"
        >
          Create first scene
        </TipButton>
      )}
      <TipButton
        className="btn"
        disabledReason={refusal ?? NARRATIVE_UNAVAILABLE.example}
        tip="Unpack the original Ashfall example into this project"
      >
        Ashfall example
      </TipButton>
    </>
  )
}

/**
 * Apply the work filters to a section that has an answer.
 *
 * Filtering is a view over the rows, so it cannot turn a failed read into an
 * empty one: only a `ready` list is narrowed, and the four other states pass
 * through untouched saying what they already said.
 */
function narrow(state: NarrativeListState, active: NarrativeFilter[]): NarrativeListState {
  if (state.kind !== 'ready' || active.length === 0) return state
  return {
    kind: 'ready',
    items: state.items.filter((item) => active.some((filter) => filter === item.status)),
  }
}

/** Namespaced, because `bands` is shared with the Library navigator's headings. */
function bandKey(id: NarrativeSectionId): string {
  return `narrative:${id}`
}
