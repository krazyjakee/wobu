import { useMemo } from 'react'
import { useNodes, useScenes } from '../../../lib/queries'

/**
 * The names behind the ids a scene document holds.
 *
 * A `Scene` refers to a character by `wobu_core::Id` and to another scene by
 * `SceneId`, because an id is what survives a rename — which is exactly the
 * property that makes a document unreadable to a person. Resolving them is a
 * *view* concern, so it lives here rather than in the adapter, and it is
 * deliberately partial: an entity nothing can name leaves its id showing rather
 * than being given a plausible one, because a made-up name is indistinguishable
 * from a real one and wrong.
 *
 * Both levels use it, which is the reason it is a hook rather than two copies:
 * the arc names the scenes an exit leads to, and a scene names its speakers and
 * the scenes it links out to. The queries are cached, so the second caller
 * costs nothing.
 */
export function useNarrativeNames(): {
  nameOf: (entity: string) => string | undefined
  sceneName: (scene: string) => string | undefined
} {
  const nodes = useNodes(true)
  const catalog = useScenes()

  const nameOf = useMemo(() => {
    const names = new Map((nodes.data ?? []).map((one) => [one.id, one.name]))
    return (entity: string) => names.get(entity)
  }, [nodes.data])

  const sceneName = useMemo(() => {
    const names = new Map((catalog.data?.scenes ?? []).map((one) => [one.id, one.name]))
    return (id: string) => names.get(id)
  }, [catalog.data])

  return useMemo(() => ({ nameOf, sceneName }), [nameOf, sceneName])
}
