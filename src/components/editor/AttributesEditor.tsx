import { useEffect, useRef, useState } from 'react'
import type { AttributeDef, WobuNode } from '../../lib/api'
import type { useAutosaveNode } from '../../hooks/useAutosaveNode'

type Autosave = ReturnType<typeof useAutosaveNode>
type AttributeMap = Record<string, unknown>

export function AttributesEditor({
  node,
  definitions,
  readOnly,
  autosave,
}: {
  node: WobuNode
  definitions: AttributeDef[]
  readOnly: boolean
  autosave: Autosave
}) {
  const queue = autosave.queue
  const [draft, setDraft] = useState<AttributeMap>(node.attributes)
  const draftRef = useRef<AttributeMap>(node.attributes)
  const storedRef = useRef<AttributeMap>(node.attributes)
  const pending = useRef(new Set<string>())
  const nodeId = useRef(node.id)

  // A refetch adopts remote attributes that are not being saved locally. The
  // autosave patch is the whole `attributes` object, so re-queue the merged
  // object when a peer changed another attribute during the debounce window;
  // otherwise one top-level patch could put that remote field back.
  useEffect(() => {
    if (nodeId.current !== node.id) {
      nodeId.current = node.id
      pending.current.clear()
      draftRef.current = node.attributes
      storedRef.current = node.attributes
      setDraft(node.attributes)
      return
    }

    const nextDraft: AttributeMap = { ...node.attributes }
    const nextStored: AttributeMap = { ...node.attributes }
    for (const key of [...pending.current]) {
      if (sameValue(node.attributes, storedRef.current, key)) {
        pending.current.delete(key)
        continue
      }
      copyKey(draftRef.current, nextDraft, key)
      copyKey(storedRef.current, nextStored, key)
    }

    draftRef.current = nextDraft
    storedRef.current = nextStored
    setDraft(nextDraft)
    if (pending.current.size > 0) queue({ attributes: nextStored })
  }, [node.id, node.attributes, queue])

  if (definitions.length === 0) return null

  const update = (definition: AttributeDef, raw: string | boolean) => {
    const nextDraft = { ...draftRef.current }
    const nextStored = { ...storedRef.current }
    pending.current.add(definition.key)

    if (definition.valueKind === 'boolean') {
      nextDraft[definition.key] = raw
      nextStored[definition.key] = raw
    } else {
      const text = raw as string
      nextDraft[definition.key] = text
      if (text.trim() === '') delete nextStored[definition.key]
      else if (definition.valueKind === 'number') {
        const number = Number(text)
        if (Number.isFinite(number)) nextStored[definition.key] = number
        else delete nextStored[definition.key]
      } else nextStored[definition.key] = text
    }

    draftRef.current = nextDraft
    storedRef.current = nextStored
    setDraft(nextDraft)
    queue({ attributes: nextStored })
  }

  return (
    <details className="attributes" open>
      <summary>
        <span>Attributes</span>
        <span className="attributes-count">{definitions.length}</span>
      </summary>
      <div className="attributes-grid">
        {definitions.map((definition) => {
          const value = draft[definition.key]
          return (
            <label className="attribute" key={definition.key}>
              <span>{definition.label}</span>
              {definition.valueKind === 'boolean' ? (
                <input
                  aria-label={definition.label}
                  type="checkbox"
                  checked={value === true}
                  disabled={readOnly}
                  onChange={(event) => update(definition, event.target.checked)}
                  onBlur={autosave.flush}
                />
              ) : (
                <input
                  aria-label={definition.label}
                  type={definition.valueKind === 'number' ? 'number' : 'text'}
                  step={definition.valueKind === 'number' ? 'any' : undefined}
                  value={scalarText(value)}
                  readOnly={readOnly}
                  spellCheck={false}
                  onChange={(event) => update(definition, event.target.value)}
                  onBlur={autosave.flush}
                />
              )}
            </label>
          )
        })}
      </div>
    </details>
  )
}

function scalarText(value: unknown): string {
  return typeof value === 'string' || typeof value === 'number' ? String(value) : ''
}

function copyKey(from: AttributeMap, to: AttributeMap, key: string) {
  if (Object.hasOwn(from, key)) to[key] = from[key]
  else delete to[key]
}

function sameValue(incoming: AttributeMap, local: AttributeMap, key: string): boolean {
  const incomingHas = Object.hasOwn(incoming, key)
  const localHas = Object.hasOwn(local, key)
  return incomingHas === localHas && (!incomingHas || Object.is(incoming[key], local[key]))
}
