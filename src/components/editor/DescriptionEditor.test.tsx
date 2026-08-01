import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { SectionDef, WobuNode } from '../../lib/api'
import { node as buildNode } from '../../test/fixtures'
import { DescriptionEditor } from './DescriptionEditor'

const definitions: SectionDef[] = [
  { key: 'silhouette', label: 'Silhouette', valueKind: 'text' },
  { key: 'anatomy', label: 'Anatomy', valueKind: 'text' },
  { key: 'palette', label: 'Palette', valueKind: 'list' },
  { key: 'signature', label: 'Signature details', valueKind: 'list' },
  { key: 'never', label: 'Never', valueKind: 'list' },
]

const autosave = {
  queue: vi.fn<(patch: Partial<WobuNode>) => void>(),
  flush: vi.fn<() => void>(),
  status: 'idle' as const,
}

const description = {
  sections: {
    silhouette: { type: 'text' as const, value: 'Tall and narrow' },
    palette: { type: 'list' as const, value: ['#2b2118', '#c2703a'] },
    signature: { type: 'list' as const, value: ['Ember-lit vents'] },
    never: { type: 'list' as const, value: ['Modern firearms'] },
    legacy_note: { type: 'text' as const, value: 'Keep this older section' },
  },
}

beforeEach(() => {
  autosave.queue.mockReset()
  autosave.flush.mockReset()
})

describe('DescriptionEditor', () => {
  it('generates prose, swatch, and list controls from the section schema', () => {
    const node = buildNode({ id: 'kael', description, descriptionState: 'fresh' })
    render(
      <DescriptionEditor
        node={node}
        definitions={definitions}
        readOnly={false}
        autosave={autosave}
      />,
    )

    expect((screen.getByLabelText('Silhouette') as HTMLTextAreaElement).value).toBe(
      'Tall and narrow',
    )
    expect((screen.getByLabelText('Anatomy') as HTMLTextAreaElement).value).toBe('')
    expect((screen.getByLabelText('Palette swatch 1') as HTMLInputElement).type).toBe('color')
    expect((screen.getByLabelText('Palette colour 1') as HTMLInputElement).value).toBe('#2b2118')
    expect((screen.getByLabelText('Signature details item 1') as HTMLInputElement).value).toBe(
      'Ember-lit vents',
    )
    expect((screen.getByLabelText('Never item 1') as HTMLInputElement).value).toBe(
      'Modern firearms',
    )
    expect((screen.getByLabelText('legacy note') as HTMLTextAreaElement).value).toBe(
      'Keep this older section',
    )
  })

  it('autosaves structured edits, marks them edited, and preserves unknown sections', () => {
    const onEdit = vi.fn()
    const node = buildNode({ id: 'kael', description, descriptionState: 'fresh' })
    render(
      <DescriptionEditor
        node={node}
        definitions={definitions}
        readOnly={false}
        autosave={autosave}
        onEdit={onEdit}
      />,
    )

    fireEvent.change(screen.getByLabelText('Silhouette'), {
      target: { value: 'Forward-canted stance' },
    })
    expect(autosave.queue).toHaveBeenLastCalledWith({
      description: {
        sections: {
          ...description.sections,
          silhouette: { type: 'text', value: 'Forward-canted stance' },
        },
      },
      descriptionState: 'edited',
    })

    fireEvent.change(screen.getByLabelText('Palette colour 1'), {
      target: { value: '#101820' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Add palette swatch' }))
    fireEvent.change(screen.getByLabelText('Palette swatch 3'), {
      target: { value: '#345678' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Remove palette colour 2' }))
    fireEvent.click(screen.getByRole('button', { name: 'Add Signature details item' }))
    fireEvent.change(screen.getByLabelText('Signature details item 2'), {
      target: { value: 'Ground-flat signet' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Remove Never item 1' }))

    expect(autosave.queue).toHaveBeenLastCalledWith({
      description: {
        sections: {
          ...description.sections,
          silhouette: { type: 'text', value: 'Forward-canted stance' },
          palette: { type: 'list', value: ['#101820', '#345678'] },
          signature: { type: 'list', value: ['Ember-lit vents', 'Ground-flat signet'] },
          never: { type: 'list', value: [] },
        },
      },
      descriptionState: 'edited',
    })
    expect(onEdit).toHaveBeenCalled()

    fireEvent.blur(screen.getByLabelText('Silhouette'))
    expect(autosave.flush).toHaveBeenCalledOnce()
  })

  it('rebases a pending section edit over an incoming edit to another section', () => {
    const node = buildNode({ id: 'kael', description, descriptionState: 'fresh' })
    const view = render(
      <DescriptionEditor
        node={node}
        definitions={definitions}
        readOnly={false}
        autosave={autosave}
      />,
    )

    fireEvent.change(screen.getByLabelText('Silhouette'), {
      target: { value: 'My unfinished silhouette' },
    })
    view.rerender(
      <DescriptionEditor
        node={{
          ...node,
          description: {
            sections: {
              ...description.sections,
              signature: { type: 'list', value: ['Incoming collaborator detail'] },
            },
          },
        }}
        definitions={definitions}
        readOnly={false}
        autosave={autosave}
      />,
    )

    expect((screen.getByLabelText('Silhouette') as HTMLTextAreaElement).value).toBe(
      'My unfinished silhouette',
    )
    expect(autosave.queue).toHaveBeenLastCalledWith({
      description: {
        sections: {
          ...description.sections,
          silhouette: { type: 'text', value: 'My unfinished silhouette' },
          signature: { type: 'list', value: ['Incoming collaborator detail'] },
        },
      },
      descriptionState: 'edited',
    })
  })

  it('keeps a new blank list row local until it contains persistable text', () => {
    const node = buildNode({ id: 'kael', description, descriptionState: 'fresh' })
    render(
      <DescriptionEditor
        node={node}
        definitions={definitions}
        readOnly={false}
        autosave={autosave}
      />,
    )

    fireEvent.click(screen.getByRole('button', { name: 'Add Signature details item' }))
    expect(autosave.queue).not.toHaveBeenCalled()

    fireEvent.blur(screen.getByLabelText('Signature details item 2'))
    expect(screen.queryByLabelText('Signature details item 2')).not.toBeInTheDocument()
    expect(autosave.flush).toHaveBeenCalledOnce()
  })

  it('makes every structured control inert in a read-only project', () => {
    const node = buildNode({ id: 'kael', description, descriptionState: 'fresh' })
    render(
      <DescriptionEditor
        node={node}
        definitions={definitions}
        readOnly={true}
        autosave={autosave}
      />,
    )

    expect((screen.getByLabelText('Silhouette') as HTMLTextAreaElement).readOnly).toBe(true)
    expect((screen.getByLabelText('Palette swatch 1') as HTMLInputElement).disabled).toBe(true)
    expect((screen.getByLabelText('Signature details item 1') as HTMLInputElement).readOnly).toBe(
      true,
    )
    expect(screen.getByRole('button', { name: 'Add Signature details item' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Remove Never item 1' })).toBeDisabled()
  })
})
