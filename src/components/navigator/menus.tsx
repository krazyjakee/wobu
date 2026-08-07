import type { NodeKind, NodeSummary } from '../../lib/api'
import { labelFor, pluralFor, type KindIndex } from '../../lib/kinds'
import type { KindGroup } from '../../lib/tree'
import { Icon } from '../Icon'
import { ContextMenu, MenuItem, MenuLabel, MenuSeparator } from '../ContextMenu'
import { Star } from './rows'
import { READ_ONLY_REASON } from './constants'

export function GroupMenu({
  group,
  x,
  y,
  opener,
  readOnly,
  allClosed,
  onClose,
  onNewNode,
  onToggleAll,
}: {
  group: KindGroup
  x: number
  y: number
  opener: HTMLElement
  readOnly: boolean
  allClosed: boolean
  onClose: () => void
  onNewNode: (kind: NodeKind | null, parentId: string | null) => void
  onToggleAll: () => void
}) {
  const plural = pluralFor(group.def, group.kind)
  return (
    <ContextMenu
      x={x}
      y={y}
      onClose={onClose}
      restoreFocus={opener}
      label={`Actions for ${plural}`}
    >
      <MenuLabel>{plural}</MenuLabel>
      <MenuItem
        icon={<Icon name="plus" size="sm" />}
        disabledReason={readOnly ? READ_ONLY_REASON : null}
        onSelect={() => onNewNode(group.kind, null)}
      >
        New {labelFor(group.def, group.kind).toLowerCase()}
      </MenuItem>
      <MenuSeparator />
      {/* The chord is printed, not typed: this is the same action as the button
          in `.nav-tools` and the same command the dispatcher runs, so the row
          says whatever the user has bound it to today. */}
      <MenuItem
        icon={<Icon name="chev" size="sm" />}
        command="nav.toggleAll"
        onSelect={onToggleAll}
      >
        {allClosed ? 'Expand everything' : 'Collapse everything'}
      </MenuItem>
    </ContextMenu>
  )
}

export function NodeMenu({
  node,
  kinds,
  readOnly,
  busy,
  favourite,
  onFavourite,
  onNewNode,
  onDuplicate,
  onDelete,
}: {
  node: NodeSummary
  kinds: KindIndex
  readOnly: boolean
  busy: boolean
  favourite: boolean
  onFavourite: (id: string) => void
  onNewNode: (kind: NodeKind | null, parentId: string | null) => void
  onDuplicate: (id: string) => void
  onDelete: (node: NodeSummary) => void
}) {
  const def = kinds.get(node.kind)
  const busyReason = busy ? 'Another change to this entity is still being saved.' : null
  return (
    <>
      <MenuLabel>{labelFor(def, node.kind)}</MenuLabel>
      <MenuItem
        icon={<Icon name="plus" size="sm" />}
        disabledReason={readOnly ? READ_ONLY_REASON : null}
        onSelect={() => onNewNode(node.kind, node.parentId)}
      >
        New {labelFor(def, node.kind).toLowerCase()}
      </MenuItem>
      {def?.nests && (
        <MenuItem
          icon={<Icon name="plus" size="sm" />}
          disabledReason={readOnly ? READ_ONLY_REASON : null}
          onSelect={() => onNewNode(node.kind, node.id)}
        >
          New child of {node.name}
        </MenuItem>
      )}
      <MenuSeparator />
      {/* Never disabled by `readOnly`: a favourite is this reader's shortcut,
          held on this machine, and a project on a read-only share is exactly
          the one you most want to keep your bearings in. */}
      <MenuItem icon={<Star on={favourite} />} onSelect={() => onFavourite(node.id)}>
        {favourite ? 'Remove from favourites' : 'Add to favourites'}
      </MenuItem>
      <MenuSeparator />
      <MenuItem
        icon={<Icon name="copy" size="sm" />}
        disabledReason={
          readOnly
            ? READ_ONLY_REASON
            : def?.singleton
              ? `A world has one ${labelFor(def, node.kind).toLowerCase()}, so there is nothing to duplicate it into.`
              : busyReason
        }
        onSelect={() => onDuplicate(node.id)}
      >
        Duplicate
      </MenuItem>
      <MenuItem
        danger
        icon={<Icon name="trash" size="sm" />}
        disabledReason={readOnly ? READ_ONLY_REASON : busyReason}
        onSelect={() => onDelete(node)}
      >
        Delete
      </MenuItem>
    </>
  )
}
