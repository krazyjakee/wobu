import { useMemo, useState } from 'react'
import type { KindDef, NodeKind, NodeSummary } from '../lib/api'
import { errorMessage } from '../lib/api'
import { useCreateNode } from '../lib/queries'
import { labelFor } from '../lib/kinds'
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
  const parents = useMemo(
    () => nodes.filter((n) => n.kind === kind).sort((a, b) => a.name.localeCompare(b.name)),
    [nodes, kind],
  )

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
            <select
              id="nn-kind"
              value={kind}
              onChange={(e) => {
                setKind(e.target.value as NodeKind)
                setParentId('')
              }}
            >
              {available.map((k) => (
                <option key={k.kind} value={k.kind}>
                  {labelFor(k, k.kind)}
                </option>
              ))}
            </select>
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
              <select id="nn-parent" value={parentId} onChange={(e) => setParentId(e.target.value)}>
                <option value="">— top level —</option>
                {parents.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
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
