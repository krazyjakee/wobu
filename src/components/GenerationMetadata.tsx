import type { Generation } from '../lib/api'
import { sceneComposition } from '../lib/api'

/** The receipt subject, preserving multi-entity scene authorship when present. */
export function GenerationSubject({
  generation,
  fallback,
}: {
  generation: Generation
  fallback: string
}) {
  const scene = sceneComposition(generation)
  return <>{scene ? `Scene · ${scene.subjectNames.join(' + ')}` : fallback}</>
}

/** The provider/model identity and immutable seed recorded by a receipt. */
export function GenerationModelSeed({
  generation,
  includeBackend = false,
}: {
  generation: Generation
  includeBackend?: boolean
}) {
  return (
    <>
      {includeBackend ? `${generation.backend} / ${generation.model}` : generation.model} · seed{' '}
      {generation.seed}
    </>
  )
}

/** Locale formatting is shared so the same receipt never shows two timestamps. */
export function GenerationTimestamp({ generation }: { generation: Generation }) {
  return <>{new Date(generation.createdAt).toLocaleString()}</>
}

/** History's second line distinguishes a scene without replacing its preset. */
export function GenerationPresetModel({ generation }: { generation: Generation }) {
  return (
    <>
      {sceneComposition(generation) ? 'Several entities · ' : ''}
      {generation.preset} · {generation.model}
    </>
  )
}
