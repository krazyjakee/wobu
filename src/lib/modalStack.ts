/**
 * How many dialogs are on top of the workspace.
 *
 * A single number, in its own module, for one reader: the keyboard dispatcher.
 * A workspace shortcut that fired while a sheet owned the screen would act on
 * something behind it — switch modes, toggle a pane — so it has to be able to
 * ask. `components/Modal.tsx` is the only writer, and it already maintains the
 * stack this mirrors; asking the DOM for `[data-modal-host]` instead would be a
 * second source of the same truth, and lives here rather than there only
 * because a component module that also exports functions breaks fast refresh.
 */

let depth = 0

export function setModalDepth(next: number): void {
  depth = next
}

export function modalOpen(): boolean {
  return depth > 0
}
