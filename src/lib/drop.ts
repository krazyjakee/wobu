import type { KindDef, NodeKind, NodeSummary } from './api'
import type { KindIndex } from './kinds'

/**
 * Everything a reparent decision depends on, gathered in one place.
 *
 * `forbidden` is passed in rather than derived because the Navigator already
 * memoises `descendantsOf(dragId)` for the duration of a drag — recomputing it
 * per hovered row would walk the whole world on every `dragover`.
 */
export interface DropContext {
  /** The node being dragged, or `null` when no drag is in progress. */
  dragId: string | null
  byId: Map<string, NodeSummary>
  /** Ids beneath `dragId` — see `descendantsOf`. */
  forbidden: Set<string>
  kinds: KindIndex
  readOnly: boolean
}

/**
 * Whether dropping the dragged node onto `targetId` is a legal reparent.
 *
 * Pure and exported rather than living inside the Navigator, because these are
 * the rules that decide whether a drag rewrites `parentId` on disk, and each
 * one of them exists to stop a specific way of corrupting the tree:
 *
 * - **Same kind only.** Nesting is within a kind by design (docs/02-data-model.md);
 *   a character under a species is not a hierarchy, it is a broken file path.
 * - **Not onto itself, not onto its own descendant.** Either one detaches the
 *   whole subtree from the roots and it stops being reachable from the tree.
 * - **Not a no-op.** Dropping a node back onto its current parent would write a
 *   file, bump `updated_at` and wake every other client on the share to say
 *   nothing changed.
 * - **Only into kinds that nest.** A kind whose registry entry says `nests:
 *   false` has no parent-child meaning at all.
 *
 * `targetId === null` means the kind's group header, i.e. "move to the top
 * level". That is legal for any nested node even when the kind does not nest —
 * a node that is already wrongly parented must always have a way out.
 */
export function canDrop(ctx: DropContext, targetId: string | null, targetKind: NodeKind): boolean {
  const { dragId, byId, forbidden, kinds, readOnly } = ctx
  if (readOnly || !dragId) return false
  const src = byId.get(dragId)
  if (!src) return false
  if (src.kind !== targetKind) return false
  if (targetId === null) return src.parentId !== null
  if (targetId === dragId || forbidden.has(targetId)) return false
  if (src.parentId === targetId) return false
  const def: KindDef | undefined = kinds.get(targetKind)
  if (def && !def.nests) return false
  return true
}
