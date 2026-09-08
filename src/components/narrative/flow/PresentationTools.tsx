import { useState } from 'react'
import { ViewportPortal, useReactFlow } from '@xyflow/react'
import type { LayoutAnnotation, NodeKey } from '../../../lib/api'
import type { FlowPresentation } from './useFlowPresentation'
import { layoutKeyOf, mintId } from './source'
import { useFlowLevel } from './flowStore'

const MAX_VISIBLE_NOTES = 40

export function PresentationTools({
  presentation,
  level,
  readOnly,
}: {
  presentation: FlowPresentation
  level: 'scene' | 'arc'
  readOnly: boolean
}) {
  const [open, setOpen] = useState(false)
  const [label, setLabel] = useState('')
  const [groupPage, setGroupPage] = useState(0)
  const selected = useFlowLevel((s) => s.selectedId)
  const flow = useReactFlow()
  const { layout, onChange } = presentation
  const key = selected ? layoutKeyOf(selected, level) : null
  const groups = Object.values(layout.groups).sort((a, b) => a.id.localeCompare(b.id))
  const currentGroupPage = Math.min(groupPage, Math.max(0, Math.ceil(groups.length / 40) - 1))
  const now = () => new Date().toISOString()
  const groupSelected = (id: string) => {
    if (!key) return
    const groups = { ...layout.groups }
    for (const [groupId, group] of Object.entries(groups)) {
      const members = (group.members ?? []).filter((member) => member !== key)
      if (groupId === id) members.push(key)
      if (members.join() !== (group.members ?? []).join())
        groups[groupId] = { ...group, members, updatedAt: now() }
    }
    onChange({ ...layout, groups })
  }
  return (
    <>
      <label className="nrt-arrangement-mode">
        Arrangement{' '}
        <select
          aria-label="Arrangement mode"
          value={layout.mode}
          disabled={readOnly}
          onChange={(event) =>
            onChange({
              ...layout,
              mode: event.target.value as 'automatic' | 'manual',
              modeUpdatedAt: now(),
            })
          }
        >
          <option value="automatic">Automatic</option>
          <option value="manual">Manual</option>
        </select>
      </label>
      {presentation.saveFailed && (
        <button
          type="button"
          className="btn btn-sm"
          disabled={readOnly}
          onClick={() => onChange(layout)}
        >
          Retry arrangement save
        </button>
      )}
      <button
        type="button"
        className="btn btn-sm"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        Groups &amp; notes
      </button>
      {open && (
        <div className="nrt-presentation-tools" aria-label="Groups and pinned notes">
          <label>
            Group name{' '}
            <input
              value={label}
              maxLength={256}
              disabled={readOnly}
              onChange={(event) => {
                if (new TextEncoder().encode(event.target.value).length <= 256)
                  setLabel(event.target.value)
              }}
            />
          </label>
          <button
            type="button"
            className="btn btn-sm"
            disabled={readOnly || !label.trim() || Object.keys(layout.groups).length >= 1000}
            onClick={() => {
              const id = mintId()
              onChange({
                ...layout,
                groups: {
                  ...layout.groups,
                  [id]: {
                    id,
                    label: label.trim(),
                    members: key ? [key] : [],
                    collapsed: false,
                    updatedAt: now(),
                  },
                },
              })
              setLabel('')
            }}
          >
            Create group
          </button>
          <label>
            Selected node group{' '}
            <select
              aria-label="Selected node group"
              disabled={readOnly || !key}
              value={
                Object.values(layout.groups).find((group) =>
                  (group.members ?? []).includes(key ?? ''),
                )?.id ?? ''
              }
              onChange={(event) => groupSelected(event.target.value)}
            >
              <option value="">Ungrouped</option>
              {Object.values(layout.groups).map((group) => (
                <option key={group.id} value={group.id}>
                  {group.label || 'Unnamed group'}
                </option>
              ))}
            </select>
          </label>
          <div className="nrt-presentation-groups">
            {groups.length > 40 && (
              <div role="status">
                Groups {currentGroupPage * 40 + 1}–
                {Math.min((currentGroupPage + 1) * 40, groups.length)} of {groups.length}
                <button
                  type="button"
                  disabled={currentGroupPage === 0}
                  onClick={() => setGroupPage(currentGroupPage - 1)}
                >
                  Previous groups
                </button>
                <button
                  type="button"
                  disabled={(currentGroupPage + 1) * 40 >= groups.length}
                  onClick={() => setGroupPage(currentGroupPage + 1)}
                >
                  Next groups
                </button>
              </div>
            )}
            {groups.slice(currentGroupPage * 40, (currentGroupPage + 1) * 40).map((group) => (
              <div key={group.id}>
                <input
                  aria-label={`Rename group ${group.label || group.id}`}
                  value={group.label ?? ''}
                  disabled={readOnly}
                  maxLength={256}
                  onChange={(event) => {
                    if (new TextEncoder().encode(event.target.value).length <= 256)
                      onChange({
                        ...layout,
                        groups: {
                          ...layout.groups,
                          [group.id]: { ...group, label: event.target.value, updatedAt: now() },
                        },
                      })
                  }}
                />
                <button
                  type="button"
                  className="btn btn-sm"
                  disabled={readOnly}
                  aria-pressed={group.collapsed ?? false}
                  onClick={() =>
                    onChange({
                      ...layout,
                      groups: {
                        ...layout.groups,
                        [group.id]: { ...group, collapsed: !group.collapsed, updatedAt: now() },
                      },
                    })
                  }
                >
                  {group.collapsed ? 'Open' : 'Close'} {group.label || 'group'}
                </button>
                <button
                  type="button"
                  className="btn btn-sm"
                  aria-label={`Delete group ${group.label ?? group.id}`}
                  disabled={readOnly}
                  onClick={() => {
                    const groups = { ...layout.groups }
                    delete groups[group.id]
                    onChange({
                      ...layout,
                      groups,
                      removedGroups: { ...layout.removedGroups, [group.id]: now() },
                    })
                  }}
                >
                  Remove
                </button>
              </div>
            ))}
          </div>
          <button
            type="button"
            className="btn btn-sm"
            disabled={readOnly || Object.keys(layout.annotations).length >= 1000}
            onClick={() => {
              const id = mintId()
              const at = flow.screenToFlowPosition({
                x: window.innerWidth / 2,
                y: window.innerHeight / 2,
              })
              const note: LayoutAnnotation = {
                id,
                body: 'New pinned note',
                ...at,
                updatedAt: now(),
                ...(key ? { attachedTo: key } : {}),
              }
              onChange({ ...layout, annotations: { ...layout.annotations, [id]: note } })
            }}
          >
            Add pinned note
          </button>
          <p className="nrt-note">
            Groups, notes and arrangement changes are saved separately from the story.
          </p>
        </div>
      )}
    </>
  )
}

/** Notes use a bounded viewport overlay, so they cannot consume React Flow's
 * 300-node store budget or turn a large layout into unbounded DOM. */
export function PinnedNotes({
  presentation,
  readOnly,
  positions,
}: {
  presentation: FlowPresentation
  readOnly: boolean
  positions: Record<NodeKey, { x: number; y: number }>
}) {
  const [page, setPage] = useState(0)
  const { layout, onChange } = presentation
  const notes = Object.values(layout.annotations).sort((a, b) => a.id.localeCompare(b.id))
  const current = Math.min(page, Math.max(0, Math.ceil(notes.length / MAX_VISIBLE_NOTES) - 1))
  const change = (note: LayoutAnnotation) =>
    onChange({ ...layout, annotations: { ...layout.annotations, [note.id]: note } })
  return (
    <>
      {notes.length > MAX_VISIBLE_NOTES && (
        <div className="nrt-note-pages" role="status">
          Showing pinned notes {current * MAX_VISIBLE_NOTES + 1}–
          {Math.min((current + 1) * MAX_VISIBLE_NOTES, notes.length)} of {notes.length}
          <button type="button" disabled={current === 0} onClick={() => setPage(current - 1)}>
            Previous notes
          </button>
          <button
            type="button"
            disabled={(current + 1) * MAX_VISIBLE_NOTES >= notes.length}
            onClick={() => setPage(current + 1)}
          >
            Next notes
          </button>
        </div>
      )}
      <ViewportPortal>
        {notes.slice(current * MAX_VISIBLE_NOTES, (current + 1) * MAX_VISIBLE_NOTES).map((note) => (
          <PinnedNote
            key={note.id}
            note={note}
            readOnly={readOnly}
            onChange={change}
            onDelete={() => {
              const annotations = { ...layout.annotations }
              delete annotations[note.id]
              onChange({
                ...layout,
                annotations,
                removedAnnotations: {
                  ...layout.removedAnnotations,
                  [note.id]: new Date().toISOString(),
                },
              })
            }}
            anchor={note.attachedTo ? positions[note.attachedTo] : undefined}
          />
        ))}
      </ViewportPortal>
    </>
  )
}
function PinnedNote({
  note,
  onChange,
  onDelete,
  readOnly,
  anchor,
}: {
  note: LayoutAnnotation
  onChange: (note: LayoutAnnotation) => void
  onDelete: () => void
  readOnly: boolean
  anchor?: { x: number; y: number }
}) {
  const [text, setText] = useState(note.body ?? '')
  const [seen, setSeen] = useState(note.body)
  if (seen !== note.body) {
    setSeen(note.body)
    setText(note.body ?? '')
  }
  const [drag, setDrag] = useState<{ startX: number; startY: number; x: number; y: number } | null>(
    null,
  )
  const flow = useReactFlow()
  return (
    <aside
      className="nrt-pinned-note nodrag nopan"
      style={{ left: drag?.x ?? note.x, top: drag?.y ?? note.y, width: note.width ?? 220 }}
      aria-label="Pinned note"
    >
      <button
        type="button"
        className="nrt-note-handle"
        aria-label="Move pinned note"
        disabled={readOnly}
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture(event.pointerId)
          setDrag({ startX: event.clientX, startY: event.clientY, x: note.x, y: note.y })
        }}
        onPointerMove={(event) => {
          if (!drag) return
          const zoom = flow.getZoom()
          setDrag({
            ...drag,
            x: note.x + (event.clientX - drag.startX) / zoom,
            y: note.y + (event.clientY - drag.startY) / zoom,
          })
        }}
        onPointerCancel={() => setDrag(null)}
        onPointerUp={() => {
          if (drag) {
            onChange({ ...note, x: drag.x, y: drag.y, updatedAt: new Date().toISOString() })
            setDrag(null)
          }
        }}
      >
        Pinned note · drag
      </button>
      <textarea
        aria-label="Pinned note text"
        value={text}
        maxLength={8192}
        disabled={readOnly}
        onChange={(event) => {
          if (new TextEncoder().encode(event.target.value).length <= 8192)
            setText(event.target.value)
        }}
        onBlur={() => {
          if (text !== note.body)
            onChange({ ...note, body: text, updatedAt: new Date().toISOString() })
        }}
      />
      <div>
        {anchor && (
          <button
            type="button"
            disabled={readOnly}
            onClick={() =>
              onChange({
                ...note,
                x: anchor.x + 30,
                y: anchor.y - 130,
                updatedAt: new Date().toISOString(),
              })
            }
          >
            Place by pinned node
          </button>
        )}
        <button
          type="button"
          aria-label="Delete pinned note"
          disabled={readOnly}
          onClick={onDelete}
        >
          Remove note
        </button>
      </div>
    </aside>
  )
}
