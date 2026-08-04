import { useMemo, useState } from 'react'
import type { KindDef, NodeKind, NodeSummary } from '../lib/api'
import { errorMessage } from '../lib/api'
import { useCreateNode } from '../lib/queries'
import { useNodeThumbs } from '../lib/nodeThumbs'
import { colorFor, labelFor, spriteFor } from '../lib/kinds'
import { NodeThumbnail } from './AssetMedia'
import { Combobox } from './Combobox'
import { Icon } from './Icon'
import { Modal } from './Modal'

export function NewNodeSheet({
  initialKind,
  initialParentId,
  nodes,
  kinds,
  onClose,
  onCreated,
}: {
  initialKind: NodeKind | null
  initialParentId: string | null
  nodes: NodeSummary[]
  kinds: KindDef[]
  onClose: () => void
  onCreated: (id: string) => void
}) {
  const create = useCreateNode()

  // Singletons already on disk cannot be created twice.
  const available = useMemo(
    () => kinds.filter((k) => !(k.singleton && nodes.some((n) => n.kind === k.kind))),
    [kinds, nodes],
  )

  const [kind, setKind] = useState<NodeKind | ''>(
    initialKind && available.some((k) => k.kind === initialKind)
      ? initialKind
      : (available[0]?.kind ?? ''),
  )
  const [name, setName] = useState('')
  const [parentId, setParentId] = useState<string>(initialParentId ?? '')
  const [err, setErr] = useState<string | null>(null)

  const def = available.find((k) => k.kind === kind)
  const parents = useMemo(() => nodes.filter((n) => n.kind === kind), [nodes, kind])
  const kindOptions = useMemo(
    () =>
      available.map((k) => ({
        value: k.kind,
        label: labelFor(k, k.kind),
        keywords: k.kind,
        icon: <Icon name={spriteFor(k, k.kind)} size="sm" style={{ color: colorFor(k, k.kind) }} />,
      })),
    [available],
  )

  /*
   * A picture on every candidate row, and on the field beside them (#146).
   *
   * This used to be a native `<select>`, which cannot draw one — an `<option>`
   * may contain text and nothing else — so the only confirmation of *which*
   * entity had been picked was the single thumbnail next to the control. The
   * shared listbox draws one per row, and `onDrawnRows` keeps that affordable:
   * only the ids in the scrolled window are asked for, so nesting inside a
   * world with three thousand characters still costs one batch of twenty.
   */
  const [drawn, setDrawn] = useState<string[]>([])
  const parentThumbs = useNodeThumbs(
    useMemo(() => [...new Set([...(parentId ? [parentId] : []), ...drawn])], [parentId, drawn]),
  )
  const kindIcon = def ? (
    <Icon name={spriteFor(def, def.kind)} size="sm" style={{ color: colorFor(def, def.kind) }} />
  ) : (
    <Icon name="cube" size="sm" />
  )

  /*
   * Rebuilt every render rather than memoised on `parents`: the thumbnail cache
   * signals a resolved picture by re-rendering, and a list memoised on the node
   * array alone would keep drawing the fallback icons it was built with.
   */
  const parentOptions = [
    { value: '', label: '— top level —', pinned: true },
    ...parents.map((p) => ({
      value: p.id,
      label: p.name,
      keywords: p.summary,
      icon: <NodeThumbnail path={parentThumbs.get(p.id)} fallback={kindIcon} />,
    })),
  ]

  function submit() {
    if (!kind) {
      setErr('Pick a kind.')
      return
    }
    if (!name.trim()) {
      setErr('Give it a name.')
      return
    }
    setErr(null)
    create.mutate(
      { kind, name: name.trim(), parentId: def?.nests && parentId ? parentId : null },
      { onError: (e) => setErr(errorMessage(e)), onSuccess: (n) => onCreated(n.id) },
    )
  }

  return (
    <Modal
      titleId="new-node-title"
      descriptionId="new-node-description"
      onClose={onClose}
      busy={create.isPending}
      busyMessage={
        create.isPending ? 'Creating the node. This operation cannot be interrupted.' : undefined
      }
    >
      <h2 id="new-node-title">New node</h2>
      <p id="new-node-description">
        Every entity is the same record — kind only selects the icon, the section schema and the
        default link roles. This writes one Markdown file under <code>nodes/</code>.
      </p>

      {available.length === 0 ? (
        <p>The kind registry is empty, so there is nothing to create.</p>
      ) : (
        <>
          <div className="field">
            <label htmlFor="nn-kind">Kind</label>
            <Combobox
              id="nn-kind"
              value={kind}
              options={kindOptions}
              sort="title"
              onChange={(next) => {
                setKind(next as NodeKind)
                setParentId('')
              }}
            />
          </div>

          <div className="field">
            <label htmlFor="nn-name">Name</label>
            <input
              id="nn-name"
              data-modal-initial-focus
              value={name}
              onChange={(e) => setName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') submit()
              }}
            />
          </div>

          {def?.nests && (
            <div className="field">
              <label htmlFor="nn-parent">Nest inside (optional)</label>
              <div className="thumb-field">
                <NodeThumbnail
                  path={parentId ? parentThumbs.get(parentId) : null}
                  fallback={kindIcon}
                />
                <Combobox
                  id="nn-parent"
                  value={parentId}
                  options={parentOptions}
                  sort="title"
                  placeholder="— top level —"
                  onChange={setParentId}
                  onDrawnRows={setDrawn}
                />
              </div>
            </div>
          )}
        </>
      )}

      {err && <div className="sheet-err">{err}</div>}

      <div className="sheet-actions">
        <button className="btn btn-ghost" onClick={onClose} disabled={create.isPending}>
          Cancel
        </button>
        <button
          className="btn btn-primary"
          onClick={submit}
          disabled={create.isPending || available.length === 0}
        >
          {create.isPending ? 'Creating…' : 'Create'}
        </button>
      </div>
    </Modal>
  )
}
