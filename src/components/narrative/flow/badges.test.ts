import { describe, expect, it } from 'vitest'
import type { NarrativeDiagnostic, Scene } from '../../../lib/api'
import {
  attachDiagnostics,
  badgeRows,
  badgeShown,
  categoryOf,
  flowDiagnostics,
  nodeOf,
  severityOf,
  ALL_BADGES,
} from './badges'
import { mintId, nodeId, sceneToFlow } from './source'

/*
 * Badges, and the three things #189 asks them to be.
 *
 * They land by *id*, they are shown as words, and no filter can hide a problem
 * that stops a release. The last one is the only assertion here that is a
 * safety property rather than a behaviour: a canvas that looks clean while a
 * route leads nowhere is worse than a canvas with no badges at all.
 */

const BEAT = mintId()
const CHOICE = mintId()
const OUTCOME = mintId()
const SLOT = mintId()

function scene(): Scene {
  return {
    id: mintId(),
    name: 'Council hearing',
    beats: [
      {
        id: BEAT,
        title: 'Arrival',
        dialogue: [{ id: SLOT, speaker: 'narrator' }],
        choices: [{ id: CHOICE, label: 'Show the logbook', to: { end: { label: '' } } }],
        outcomes: [{ id: OUTCOME, to: { beat: BEAT } }],
      },
    ],
  }
}

function diagnostic(over: Partial<NarrativeDiagnostic>): NarrativeDiagnostic {
  return { kind: 'beat', code: 'no_destination', message: 'x', destination: false, ...over }
}

describe('a badge lands by id, never by text or position', () => {
  it('puts a choice’s problem on the choice, and an outcome’s on the outcome', () => {
    // The ids the Rust side already keys `Site::Destination(DestinationSite::…)`
    // by, straight through `commands/narrative.rs`'s flat view. Nothing here
    // reads a message or a title, so rewording a diagnostic cannot move a badge.
    expect(nodeOf(diagnostic({ kind: 'choice', beatId: BEAT, choiceId: CHOICE }))).toBe(
      nodeId.choice(CHOICE),
    )
    expect(nodeOf(diagnostic({ kind: 'outcome', beatId: BEAT, outcomeId: OUTCOME }))).toBe(
      nodeId.outcome(OUTCOME),
    )
  })

  it('puts a slot’s and a variant’s problem on the beat that holds them', () => {
    // One beat is one node however many lines are under it, so there is no box
    // for a slot to land on — and the beat is the box a writer would open.
    expect(nodeOf(diagnostic({ kind: 'dialogueSlot', beatId: BEAT, slotId: SLOT }))).toBe(
      nodeId.beat(BEAT),
    )
    expect(nodeOf(diagnostic({ kind: 'variant', beatId: BEAT, slotId: SLOT }))).toBe(
      nodeId.beat(BEAT),
    )
  })

  it('gives a problem with the scene itself no box, rather than the wrong one', () => {
    expect(nodeOf(diagnostic({ kind: 'scene', code: 'no_beats' }))).toBeNull()
    expect(nodeOf(diagnostic({ kind: 'entry', code: 'type_error' }))).toBeNull()
  })

  it('attaches to the level, and keeps the scene-wide ones separate', () => {
    const doc = scene()
    const found = attachDiagnostics(sceneToFlow(doc), [
      diagnostic({
        kind: 'choice',
        code: 'dangling_beat',
        beatId: BEAT,
        choiceId: CHOICE,
        destination: true,
        message: 'nowhere',
      }),
      diagnostic({
        kind: 'dialogueSlot',
        code: 'missing_text',
        beatId: BEAT,
        slotId: SLOT,
        message: 'no text',
      }),
      diagnostic({ kind: 'scene', code: 'no_beats', message: 'this scene has no beats' }),
    ])
    const choice = found.level.elements.find((one) => one.id === nodeId.choice(CHOICE))
    expect(choice?.diagnostics?.map((one) => one.code)).toEqual(['dangling_beat'])
    // A destination problem marks the wire out as well as the box, so the port
    // row can wear it.
    expect(choice?.diagnostics?.[0]?.portId).toBe('then')
    const beat = found.level.elements.find((one) => one.id === nodeId.beat(BEAT))
    expect(beat?.diagnostics?.map((one) => one.code)).toEqual(['missing_text'])
    expect(beat?.diagnostics?.[0]?.slotId).toBe(SLOT)
    expect(found.sceneWide.map((one) => one.code)).toEqual(['no_beats'])
  })

  it('keeps a finding whose element is not drawn rather than dropping it', () => {
    const doc = scene()
    const found = attachDiagnostics(sceneToFlow(doc), [
      diagnostic({ kind: 'beat', beatId: mintId(), message: 'a beat that is not on this canvas' }),
    ])
    expect(found.unplaced).toHaveLength(1)
  })
})

describe('severity is a product judgement, written down', () => {
  it('blocks a release on the things the story cannot do', () => {
    for (const code of [
      'dangling_beat',
      'deleted_beat',
      'unknown_scene',
      'no_destination',
      'type_error',
      'duplicate_id',
      'revision_mismatch',
    ]) {
      expect(severityOf(code)).toBe('error')
    }
    // Missing prose is a task, not a broken scene: #151 says so in as many
    // words, and the release gate that refuses to ship one is the CLI's.
    expect(severityOf('missing_text')).toBe('warning')
  })

  it('treats a code it has never heard of as an error', () => {
    // A newer Wobu can report something this build cannot classify. Hiding it
    // is the only mistake here nobody would notice, so it is not available.
    expect(severityOf('a_problem_from_the_future')).toBe('error')
    expect(categoryOf('a_problem_from_the_future')).toBe('other')
  })
})

describe('no filter can hide a release-blocking error', () => {
  it('shows an error with every filter off', () => {
    const off = {
      warnings: false,
      categories: {
        destination: false,
        reference: false,
        text: false,
        identity: false,
        type: false,
        other: false,
      },
    }
    const error = {
      id: 'a',
      code: 'dangling_beat',
      message: 'nowhere',
      severity: 'error' as const,
      category: 'destination' as const,
    }
    const warning = {
      ...error,
      id: 'b',
      code: 'missing_text',
      severity: 'warning' as const,
      category: 'text' as const,
    }
    expect(badgeShown(error, off)).toBe(true)
    expect(badgeShown(warning, off)).toBe(false)
    expect(badgeShown(warning, ALL_BADGES)).toBe(true)
  })

  it('draws a badge as a count and a word, never as a colour alone', () => {
    const rows = badgeRows(
      [
        {
          id: 'a',
          code: 'dangling_beat',
          message: 'one',
          severity: 'error',
          category: 'destination',
        },
        {
          id: 'b',
          code: 'no_destination',
          message: 'two',
          severity: 'error',
          category: 'destination',
        },
        { id: 'c', code: 'missing_text', message: 'three', severity: 'warning', category: 'text' },
      ],
      ALL_BADGES,
    )
    expect(rows.map((row) => row.label)).toEqual(['2 destination errors', '1 text warning'])
    // The full wording travels on the badge rather than being lost: a box has
    // no room for three sentences, and a box that grew to hold them would make
    // layout a function of how broken the scene is.
    expect(rows[0]?.detail).toBe('one · two')
    expect(rows.every((row) => row.icon.length > 0)).toBe(true)
  })
})

describe('what the canvas does with the attached findings', () => {
  it('gives the canvas an element id per error, which is what forbids muting it', () => {
    const doc = scene()
    const found = attachDiagnostics(sceneToFlow(doc), [
      diagnostic({
        kind: 'choice',
        code: 'dangling_beat',
        beatId: BEAT,
        choiceId: CHOICE,
        destination: true,
        message: 'nowhere',
      }),
      diagnostic({
        kind: 'dialogueSlot',
        code: 'missing_text',
        beatId: BEAT,
        slotId: SLOT,
        message: 'no text',
      }),
    ])
    const flow = flowDiagnostics(found.level)
    expect(flow.filter((one) => one.severity === 'error').map((one) => one.elementId)).toEqual([
      nodeId.choice(CHOICE),
    ])
    expect(flow.find((one) => one.severity === 'error')?.field).toBe(
      `${nodeId.choice(CHOICE)}.then`,
    )
  })
})
