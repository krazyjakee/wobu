import { createFlowStore } from '../flowStore'

/**
 * The arc level's canvas state: its cursor, its closed quests, its viewport.
 *
 * Module scope, and deliberately not created inside a component. Entering a
 * scene **unmounts** the arc canvas — which is what keeps one level's worth of
 * nodes in React Flow's store rather than two, and the spike's budget is 300
 * nodes *in the store* — so a store created per mount would go with it, taking
 * the selection, the opened quests and the viewport. Living out here is the
 * whole mechanism by which coming back lands where the writer left.
 *
 * In its own file because `ArcFlow.tsx` exports components, and a module that
 * exports both a component and a live value cannot be hot-reloaded: swapping it
 * would silently mint a second store and the two canvases would disagree.
 */
export const ARC_STORE = createFlowStore()
