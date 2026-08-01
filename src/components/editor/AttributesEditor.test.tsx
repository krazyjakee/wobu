import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { AttributeDef, WobuNode } from '../../lib/api'
import { node as buildNode } from '../../test/fixtures'
import { AttributesEditor } from './AttributesEditor'

const definitions: AttributeDef[] = [
  { key: 'era', label: 'Era', valueKind: 'text' },
  { key: 'scale', label: 'Scale', valueKind: 'number' },
  { key: 'inhabited', label: 'Inhabited', valueKind: 'boolean' },
]

const autosave = {
  queue: vi.fn<(patch: Partial<WobuNode>) => void>(),
  flush: vi.fn<() => void>(),
  status: 'idle' as const,
}

beforeEach(() => {
  autosave.queue.mockReset()
  autosave.flush.mockReset()
})

describe('AttributesEditor', () => {
  it('generates controls from the registry and autosaves typed values', () => {
    const node = buildNode({
      id: 'kael',
      attributes: { era: 'old', scale: 3, inhabited: true, legacy: 'preserved' },
    })
    render(
      <AttributesEditor
        node={node}
        definitions={definitions}
        readOnly={false}
        autosave={autosave}
      />,
    )

    const era = screen.getByLabelText('Era') as HTMLInputElement
    const scale = screen.getByLabelText('Scale') as HTMLInputElement
    const inhabited = screen.getByLabelText('Inhabited') as HTMLInputElement
    expect(era.type).toBe('text')
    expect(era.value).toBe('old')
    expect(scale.type).toBe('number')
    expect(scale.value).toBe('3')
    expect(inhabited.type).toBe('checkbox')
    expect(inhabited.checked).toBe(true)

    fireEvent.change(era, { target: { value: 'modern' } })
    fireEvent.change(scale, { target: { value: '4.5' } })
    fireEvent.click(inhabited)

    expect(autosave.queue).toHaveBeenLastCalledWith({
      attributes: { era: 'modern', scale: 4.5, inhabited: false, legacy: 'preserved' },
    })

    fireEvent.change(era, { target: { value: '' } })
    expect(autosave.queue).toHaveBeenLastCalledWith({
      attributes: { scale: 4.5, inhabited: false, legacy: 'preserved' },
    })

    fireEvent.blur(scale)
    expect(autosave.flush).toHaveBeenCalledOnce()
  })

  it('merges remote changes to undeclared attributes around a pending local edit', () => {
    const node = buildNode({ id: 'kael', attributes: { era: 'old', legacy: 'first' } })
    const view = render(
      <AttributesEditor
        node={node}
        definitions={definitions}
        readOnly={false}
        autosave={autosave}
      />,
    )

    fireEvent.change(screen.getByLabelText('Era'), { target: { value: 'modern' } })
    view.rerender(
      <AttributesEditor
        node={{ ...node, attributes: { era: 'old', legacy: 'incoming' } }}
        definitions={definitions}
        readOnly={false}
        autosave={autosave}
      />,
    )

    expect((screen.getByLabelText('Era') as HTMLInputElement).value).toBe('modern')
    expect(autosave.queue).toHaveBeenLastCalledWith({
      attributes: { era: 'modern', legacy: 'incoming' },
    })
  })

  it('honours read-only projects and renders nothing for kinds without attributes', () => {
    const node = buildNode({ id: 'kael', attributes: { era: 'old', inhabited: true } })
    const view = render(
      <AttributesEditor
        node={node}
        definitions={definitions}
        readOnly={true}
        autosave={autosave}
      />,
    )

    expect((screen.getByLabelText('Era') as HTMLInputElement).readOnly).toBe(true)
    expect((screen.getByLabelText('Inhabited') as HTMLInputElement).disabled).toBe(true)

    view.rerender(
      <AttributesEditor node={node} definitions={[]} readOnly={false} autosave={autosave} />,
    )
    expect(screen.queryByText('Attributes')).not.toBeInTheDocument()
  })
})
