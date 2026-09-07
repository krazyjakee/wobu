import type { NarrativeFilter } from '../../store/ui'

/**
 * The vocabulary the Narrative shell draws with, and the reasons it gives for
 * what it cannot do yet.
 *
 * Nothing here is data. Scenes are real — they are read from the project folder
 * — but the models behind quests, world state and the text library are not
 * built, and neither is the compiler, so those lists render an *unavailable*
 * state rather than an invented one. The reasons live together in one object
 * because they are the part a reader will actually see, and scattering them
 * through eight components is how a refused button ends up saying nothing at
 * all.
 */

export type NarrativeSectionId = 'scenes' | 'quests' | 'world' | 'text'

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

/** One row of a Library section. Identified, never positioned. */
export interface NarrativeItem {
  id: string
  name: string
  status?: NarrativeStatus
  /**
   * Set when this element has an unresolved conflict on disk. The row stays
   * selectable: a conflict is a thing to open and resolve, not a thing to hide.
   */
  conflict?: string
}

/**
 * What a list can be, including the two answers that are not a list.
 *
 * `ready` with no items is the *empty* state and is a real answer — the project
 * has no scenes yet — which is why it is not a separate variant: only an empty
 * `ready` may offer to create the first one. `unavailable` is the different
 * claim that Wobu cannot answer the question at all.
 */
export type NarrativeListState =
  | { kind: 'unavailable'; reason: string }
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; items: NarrativeItem[] }

export interface NarrativeSectionDef {
  id: NarrativeSectionId
  label: string
  /** Said in the empty state, so a writer knows what belongs in the section. */
  empty: string
}

export const NARRATIVE_SECTIONS: NarrativeSectionDef[] = [
  {
    id: 'scenes',
    label: 'Scenes',
    empty: 'A scene is a place in the story where people speak and choices are made.',
  },
  {
    id: 'quests',
    label: 'Quests',
    empty: 'A quest groups the scenes of one thread and the state that drives it.',
  },
  {
    id: 'world',
    label: 'World state',
    empty: 'Facts, variables, and who knows what — the canon a scene is allowed to draw on.',
  },
  {
    id: 'text',
    label: 'Text library',
    empty: 'Barks, ambient exchanges, codex entries and other text with no player choice in it.',
  },
]

/**
 * Why each control is refused today.
 *
 * Written as what is missing rather than as "coming soon": a reader deciding
 * whether they have hit a bug is owed the actual reason, and none of these is
 * a failure they can retry.
 */
export const NARRATIVE_UNAVAILABLE = {
  source:
    'The narrative model this build reads is scenes, their beats, and the choices and outcomes between them. Facts, knowledge, relationships and quests are separate work, so anything that needs one of those is refused here rather than guessed at.',
  create:
    'Wobu cannot create one of these yet — the model that would say what it is does not exist in this build. Scenes are the part that does.',
  example:
    'Unpacking the Ashfall example needs a command that writes a whole project of scenes, world state and characters at once. This build has the one that writes a single scene, which is not the same thing.',
  search:
    'Finding a scene or a line by name needs narrative source in the local index, and scenes are not in it yet (#153). Until they are, the Scenes list below is read from the folder itself and is the way to find one.',
  quests:
    'Quests are not in this build: nothing in a project declares one, so Wobu cannot say which quest a scene belongs to or what state it is in. Grouping by quest needs that model, and guessing one from scene names would look like an answer without being one.',
  flow: 'No scene is selected, so there is nothing to draw. Choose one in the Library, or open one from the arc.',
  /**
   * #189 asks a badge to open a witness scenario in Preview. There is no
   * Preview and no runtime to produce a witness with, so the affordance says so
   * instead of opening something that would have to be fabricated.
   */
  witness:
    'A badge cannot open a witness scenario yet: that needs the deterministic runtime and the Preview overlay (#158, #161, #188), and neither is in this build. The diagnostics below are read from the source, which is a different and weaker claim than "this state is reachable".',
  /**
   * #189 also asks for affected-build scope highlighting. That needs the
   * dependency tracker, which decides what a change invalidated.
   */
  affectedScope:
    'Highlighting the beats in an affected-build scope needs the dependency tracker and the build planner (#168, #169), which are not in this build, so no scope can be selected here.',
  script:
    'The dialogue editor is not in this build. It will write, compare and review the lines of whichever beat is selected.',
  preview:
    'Preview is not in this build. It will play the selected scene from a chosen state and show which branch ran and why.',
  sourceView:
    'The source view is not in this build. It will edit the same scene as structured YAML, with errors pointing back at the form field they came from.',
  review:
    'Review compares drafts against the text they would replace, and this build has neither. Nothing has been generated, so there is nothing queued.',
  build:
    'Build works out which content a change affected before anything runs. It needs compiled narrative source, which this build does not produce.',
  export:
    'Export packages a compiled story for a game engine. This build has no compiler, so there is nothing to package.',
  diagnostics:
    'These are read from the scene source: destinations that do not resolve, conditions and effects that do not type, duplicated ids, revisions that no longer describe their words, and slots still waiting for text. They are not a compilation (#158) and they are not a reachability proof — a beat no state can reach is not something this build can find.',
} as const

/**
 * The default for every Library section, and the honest answer for the three
 * that keep it.
 *
 * `NarrativeMode` replaces the Scenes entry with the project's own catalog. The
 * other three stay: quests, world state and the text library have no model in
 * this build, and an empty list would say "this project has none of these",
 * which is a different and false claim.
 */
export const NO_NARRATIVE_SOURCE = Object.fromEntries(
  NARRATIVE_SECTIONS.map((section) => [
    section.id,
    { kind: 'unavailable', reason: NARRATIVE_UNAVAILABLE.source },
  ]),
) as Record<NarrativeSectionId, NarrativeListState>
