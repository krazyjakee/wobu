import { create } from 'zustand'

/** Ephemeral reveal, consumed by Script; never a second source document. */
export const useSceneLibrary = create<{
  searchVariant: { sceneId: string; slotId: string; variantId: string } | null
}>()(() => ({ searchVariant: null }))
