import type { ArcScene } from '../../../../lib/api/narrativeArc'
import type { Quest } from '../../../../lib/api/narrativeWorld'
import type { Scene } from '../../../../lib/api'
import type { FlowElement, FlowSceneNode } from '../model'
import { conditionText } from '../source'
import type { FlowArc } from './model'

export const questStageId = (quest: string, stage: string) => `stage:${quest}:${stage}`

/** Deterministic presentation IDs, never source identities or security hashes (FNV-1a128). */
export function groupIdentity(value: string): string {
  let hash = 0x6c62272e07bb014262b821756295c58dn
  for (const byte of new TextEncoder().encode(value))
    hash = BigInt.asUintN(128, (hash ^ BigInt(byte)) * 0x1000000000000000000013bn)
  let id = ''
  for (let index = 0; index < 26; index++) {
    id = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'[Number(hash & 31n)] + id
    hash >>= 5n
  }
  return id
}

export function draftArcScene(scene: Scene, base: ArcScene): ArcScene {
  const beats = scene.beats ?? []
  const slots = beats.flatMap((beat) => beat.dialogue ?? [])
  const variants = slots.flatMap((slot) => slot.variants ?? [])
  return {
    ...base,
    summary: { ...base.summary, name: scene.name },
    participants: (scene.participants ?? []).map((person) => person.entity),
    beats: beats.length,
    slots: slots.length,
    filled: slots.filter((slot) => slot.variants?.some((v) => v.text.body.trim())).length,
    counts: {
      ...base.counts,
      needsReview: variants.filter((v) => v.text.lifecycle?.review !== 'approved').length,
      outOfDate: variants.filter((v) => v.text.lifecycle?.freshness === 'out_of_date').length,
      locked: slots.reduce(
        (count, slot) =>
          count +
          (slot.variants ?? []).filter(
            (v) => slot.policy === 'locked' || v.text.lifecycle?.policy === 'locked',
          ).length,
        0,
      ),
    },
    arc: {
      needsText:
        beats.filter((beat) => !beat.dialogue?.length).length +
        slots.filter(
          (slot) => !slot.variants?.length || slot.variants.some((v) => !v.text.body.trim()),
        ).length,
      exits: beats.flatMap((beat) =>
        [
          ...(beat.choices ?? []).map((route) => ({ route, choice: true, label: route.label })),
          ...(beat.outcomes ?? []).map((route) => ({ route, choice: false, label: 'Outcome' })),
        ].flatMap(({ route, choice, label }) =>
          'scene' in route.to || 'unresolved' in route.to
            ? [
                {
                  beatId: beat.id,
                  routeId: route.id,
                  choice,
                  label,
                  to: 'scene' in route.to ? route.to.scene : null,
                },
              ]
            : [],
        ),
      ),
    },
  }
}

export function projectArc(
  scenes: ArcScene[],
  quests: Quest[] | undefined,
  scope: string,
  nameOf: (id: string) => string | undefined,
): FlowArc {
  const quest = quests?.find((one) => one.id === scope)
  const members = quest ? new Set(quest.scene_ids) : null
  // Authored destinations outside a quest stay visible as boundary scenes.
  const wanted = members ? new Set(members) : null
  if (wanted) {
    const byId = new Map(scenes.map((scene) => [scene.summary.id, scene]))
    const queue = [...wanted]
    for (let index = 0; index < queue.length; index++) {
      for (const exit of byId.get(queue[index]!)?.arc.exits ?? []) {
        if (exit.to && !wanted.has(exit.to)) {
          wanted.add(exit.to)
          queue.push(exit.to)
        }
      }
    }
  }
  const elements: FlowElement[] = scenes
    .filter((one) => !wanted || wanted.has(one.summary.id))
    .map((one): FlowSceneNode => ({
      id: one.summary.id,
      kind: 'scene',
      title: one.summary.name,
      participants: one.participants,
      participantLabels: Object.fromEntries(one.participants.map((id) => [id, nameOf(id) ?? id])),
      questIds: (quests ?? []).filter((q) => q.scene_ids.includes(one.summary.id)).map((q) => q.id),
      counts: {
        beats: one.beats,
        needsText: one.arc.needsText,
        needsReview: one.counts.needsReview,
        outOfDate: one.counts.outOfDate,
        locked: one.counts.locked,
      },
      out: one.arc.exits.map((exit) => ({
        id: `${exit.choice ? 'choice' : 'outcome'}:${exit.routeId}`,
        label: exit.label,
        to: exit.to,
        via: exit.beatId,
        routeId: exit.routeId,
        choice: exit.choice,
      })),
    }))
  for (const one of quests ?? []) {
    if (quest && one.id !== quest.id) continue
    for (const stage of one.stages)
      elements.push({
        id: questStageId(one.id, stage),
        kind: 'questStage',
        title: `${one.name} · ${stage}`,
        questId: one.id,
        stage,
        derived: true,
        out: one.transitions.flatMap((transition, index) =>
          transition.from === stage
            ? [
                {
                  id: `transition:${index}`,
                  label: `${transition.from} → ${transition.to}`,
                  to: questStageId(one.id, transition.to),
                  fixed: true,
                  requires: conditionText(transition.when),
                },
              ]
            : [],
        ),
      })
    // An undeclared source stage must not erase an authored transition.
    for (const stage of new Set(
      one.transitions
        .map((transition) => transition.from)
        .filter((stage) => !one.stages.includes(stage)),
    ))
      elements.push({
        id: questStageId(one.id, stage),
        kind: 'questStage',
        title: `${one.name} · Missing stage ${stage}`,
        questId: one.id,
        stage,
        derived: true,
        out: one.transitions.flatMap((transition, index) =>
          transition.from === stage
            ? [
                {
                  id: `transition:${index}`,
                  label: `${transition.from} → ${transition.to}`,
                  to: questStageId(one.id, transition.to),
                  fixed: true,
                },
              ]
            : [],
        ),
      })
  }
  return {
    level: {
      id: quest ? `quest:${quest.id}` : 'arc:project',
      name: quest?.name ?? 'Every scene',
      elements,
      groups: [],
      entryId: null,
    },
    quests:
      quests?.map((q) => ({
        id: q.id,
        name: q.name,
        state: q.initial || null,
        stages: q.stages,
        transitions: q.transitions,
      })) ?? null,
  }
}
