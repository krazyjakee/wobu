import { useEffect, useRef, useState } from 'react'
import type { SectionDef, SectionValue, WobuNode } from '../../lib/api'
import type { useAutosaveNode } from '../../hooks/useAutosaveNode'

type Autosave = ReturnType<typeof useAutosaveNode>
type SectionMap = Record<string, SectionValue>

export function DescriptionEditor({
  node,
  definitions,
  readOnly,
  autosave,
  onEdit,
}: {
  node: WobuNode
  definitions: SectionDef[]
  readOnly: boolean
  autosave: Autosave
  onEdit?: () => void
}) {
  const queue = autosave.queue
  const incoming = node.description?.sections ?? {}
  const [draft, setDraft] = useState<SectionMap>(incoming)
  const draftRef = useRef<SectionMap>(incoming)
  const storedRef = useRef<SectionMap>(incoming)
  const pending = useRef(new Set<string>())
  const nodeId = useRef(node.id)

  // Description is one top-level autosave field. Rebase locally edited
  // sections over a refetch so a peer's change to a different section is not
  // put back when this whole object is eventually written.
  useEffect(() => {
    if (nodeId.current !== node.id) {
      nodeId.current = node.id
      pending.current.clear()
      draftRef.current = incoming
      storedRef.current = incoming
      setDraft(incoming)
      return
    }

    const nextDraft: SectionMap = { ...incoming }
    const nextStored: SectionMap = { ...incoming }
    for (const key of [...pending.current]) {
      if (sameSection(incoming, storedRef.current, key)) {
        pending.current.delete(key)
        continue
      }
      copySection(draftRef.current, nextDraft, key)
      copySection(storedRef.current, nextStored, key)
    }

    draftRef.current = nextDraft
    storedRef.current = nextStored
    setDraft(nextDraft)
    if (pending.current.size > 0) {
      queue({ description: { sections: nextStored }, descriptionState: 'edited' })
    }
  }, [node.id, node.description, queue])

  const update = (key: string, value: SectionValue) => {
    const nextDraft = { ...draftRef.current, [key]: value }
    const nextStored = { ...storedRef.current, [key]: persistedValue(value) }
    pending.current.add(key)
    draftRef.current = nextDraft
    storedRef.current = nextStored
    setDraft(nextDraft)
    onEdit?.()
    queue({ description: { sections: nextStored }, descriptionState: 'edited' })
  }

  const updateDraft = (key: string, value: SectionValue) => {
    const nextDraft = { ...draftRef.current, [key]: value }
    draftRef.current = nextDraft
    setDraft(nextDraft)
  }

  return (
    <div className="description-editor">
      {orderedSections(definitions, draft).map((definition) => {
        const value = valueFor(definition, draft)
        return (
          <section
            key={definition.key}
            className={`sec sec-edit${definition.key === 'never' ? ' never' : ''}`}
          >
            <h3>{definition.label}</h3>
            {definition.key === 'palette' && value.type === 'list' ? (
              <PaletteEditor
                value={value.value}
                readOnly={readOnly}
                onChange={(items) => update(definition.key, { type: 'list', value: items })}
                onBlur={autosave.flush}
              />
            ) : value.type === 'list' ? (
              <ListEditor
                section={definition}
                value={value.value}
                readOnly={readOnly}
                onChange={(items) => update(definition.key, { type: 'list', value: items })}
                onDraftChange={(items) =>
                  updateDraft(definition.key, { type: 'list', value: items })
                }
                onBlur={autosave.flush}
              />
            ) : (
              <textarea
                className="description-text"
                aria-label={definition.label}
                value={value.value}
                readOnly={readOnly}
                spellCheck={false}
                rows={3}
                onChange={(event) =>
                  update(definition.key, { type: 'text', value: event.target.value })
                }
                onBlur={autosave.flush}
              />
            )}
          </section>
        )
      })}
    </div>
  )
}

function PaletteEditor({
  value,
  readOnly,
  onChange,
  onBlur,
}: {
  value: string[]
  readOnly: boolean
  onChange: (value: string[]) => void
  onBlur: () => void
}) {
  const replace = (index: number, item: string) => {
    const next = [...value]
    next[index] = item
    onChange(next)
  }

  return (
    <div className="palette-editor">
      <div className="palette-swatches">
        {value.map((item, index) => (
          <div className="palette-swatch" key={`palette-${index}`}>
            <input
              className="palette-picker"
              aria-label={`Palette swatch ${index + 1}`}
              type="color"
              value={isHexColour(item) ? item : '#000000'}
              disabled={readOnly}
              onChange={(event) => replace(index, event.target.value)}
              onBlur={onBlur}
            />
            <input
              className="palette-value"
              aria-label={`Palette colour ${index + 1}`}
              value={item}
              readOnly={readOnly}
              spellCheck={false}
              onChange={(event) => replace(index, event.target.value)}
              onBlur={onBlur}
            />
            <button
              className="description-remove"
              type="button"
              aria-label={`Remove palette colour ${index + 1}`}
              disabled={readOnly}
              onClick={() => onChange(value.filter((_, itemIndex) => itemIndex !== index))}
            >
              ×
            </button>
          </div>
        ))}
      </div>
      <button
        className="description-add"
        type="button"
        aria-label="Add palette swatch"
        disabled={readOnly}
        onClick={() => onChange([...value, '#000000'])}
      >
        + Add swatch
      </button>
    </div>
  )
}

function ListEditor({
  section,
  value,
  readOnly,
  onChange,
  onDraftChange,
  onBlur,
}: {
  section: SectionDef
  value: string[]
  readOnly: boolean
  onChange: (value: string[]) => void
  onDraftChange: (value: string[]) => void
  onBlur: () => void
}) {
  const replace = (index: number, item: string) => {
    const next = [...value]
    next[index] = item
    onChange(next)
  }

  return (
    <div className="description-list">
      {value.map((item, index) => (
        <div className="description-list-row" key={`${section.key}-${index}`}>
          <input
            aria-label={`${section.label} item ${index + 1}`}
            value={item}
            readOnly={readOnly}
            spellCheck={false}
            onChange={(event) => replace(index, event.target.value)}
            onBlur={() => {
              if (item.trim() === '') {
                onDraftChange(value.filter((_, itemIndex) => itemIndex !== index))
              }
              onBlur()
            }}
          />
          <button
            className="description-remove"
            type="button"
            aria-label={`Remove ${section.label} item ${index + 1}`}
            disabled={readOnly}
            onClick={() => {
              const next = value.filter((_, itemIndex) => itemIndex !== index)
              if (item.trim() === '') onDraftChange(next)
              else onChange(next)
            }}
          >
            ×
          </button>
        </div>
      ))}
      <button
        className="description-add"
        type="button"
        aria-label={`Add ${section.label} item`}
        disabled={readOnly}
        onClick={() => onDraftChange([...value, ''])}
      >
        + Add item
      </button>
    </div>
  )
}

function orderedSections(definitions: SectionDef[], sections: SectionMap): SectionDef[] {
  const ordered = [...definitions]
  const seen = new Set(definitions.map((definition) => definition.key))
  for (const [key, value] of Object.entries(sections)) {
    if (!seen.has(key)) {
      ordered.push({ key, label: key.replace(/_/g, ' '), valueKind: value.type })
    }
  }
  return ordered
}

function valueFor(definition: SectionDef, sections: SectionMap): SectionValue {
  const value = sections[definition.key]
  if (value?.type === definition.valueKind) return value
  return definition.valueKind === 'list'
    ? { type: 'list', value: [] }
    : { type: 'text', value: '' }
}

function persistedValue(value: SectionValue): SectionValue {
  return value.type === 'text'
    ? { type: 'text', value: value.value }
    : {
        type: 'list',
        value: value.value.filter((item) => item.trim() !== ''),
      }
}

function copySection(from: SectionMap, to: SectionMap, key: string) {
  if (Object.hasOwn(from, key)) to[key] = from[key]!
  else delete to[key]
}

function sameSection(incoming: SectionMap, local: SectionMap, key: string): boolean {
  const incomingHas = Object.hasOwn(incoming, key)
  const localHas = Object.hasOwn(local, key)
  if (incomingHas !== localHas) return false
  if (!incomingHas) return true
  const left = incoming[key]!
  const right = local[key]!
  if (left.type !== right.type) return false
  if (left.type === 'text' && right.type === 'text') return left.value === right.value
  if (left.type === 'list' && right.type === 'list') {
    return (
      left.value.length === right.value.length &&
      left.value.every((item, index) => item === right.value[index])
    )
  }
  return false
}

function isHexColour(value: string): boolean {
  return /^#[0-9a-f]{6}$/i.test(value)
}
