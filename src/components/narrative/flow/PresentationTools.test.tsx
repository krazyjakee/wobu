import { useEffect, useState } from 'react'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import type { Layout } from '../../../lib/api'
import { FlowCanvas } from './FlowCanvas'
import { chainScene } from './fixture'
import { resetFlowStore } from './flowStore'
import { mintId } from './source'

const h = vi.hoisted(() => ({ count: (): number => 0 }))
vi.mock('@xyflow/react', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@xyflow/react')>()
  function MeasuredFlow(props: React.ComponentProps<typeof actual.ReactFlow>) {
    const store = actual.useStoreApi()
    useEffect(() => {
      h.count = () => store.getState().nodes.length
    }, [store])
    return <actual.ReactFlow {...props} />
  }
  return { ...actual, ReactFlow: MeasuredFlow }
})

beforeEach(() => {
  resetFlowStore()
  onScene.mockClear()
})
const scene = chainScene(300, 3)
const onScene = vi.fn()
const at = '2026-09-08T12:00:00Z'
function Harness({ readOnly = false }: { readOnly?: boolean }) {
  const [layout, setLayout] = useState<Layout>(() => ({
    schemaVersion: 2,
    graph: { kind: 'scene', scene: mintId() },
    mode: 'manual',
    modeUpdatedAt: at,
    nodes: {},
    groups: Object.fromEntries(
      Array.from({ length: 45 }, (_, i) => {
        const id = mintId()
        return [id, { id, label: `Group ${i + 1}`, updatedAt: at }]
      }),
    ),
    annotations: Object.fromEntries(
      Array.from({ length: 1000 }, (_, i) => {
        const id = mintId()
        return [id, { id, body: `Note ${i + 1}`, x: 0, y: 0, updatedAt: at }]
      }),
    ),
  }))
  return (
    <FlowCanvas
      scene={scene}
      onChange={onScene}
      readOnly={readOnly}
      presentation={{ layout, onChange: setLayout }}
    />
  )
}

it('bounds actual React Flow store including frames while paging notes and groups', async () => {
  render(<Harness />)
  await waitFor(() => expect(h.count()).toBeGreaterThan(0))
  expect(h.count()).toBeLessThanOrEqual(300)
  expect(screen.getAllByRole('textbox', { name: 'Pinned note text' })).toHaveLength(40)
  fireEvent.click(screen.getByRole('button', { name: 'Next notes' }))
  expect(screen.getByText(/Showing pinned notes 41–80 of 1000/)).toBeInTheDocument()
  expect(h.count()).toBeLessThanOrEqual(300)
  fireEvent.click(screen.getByRole('button', { name: 'Groups & notes' }))
  expect(screen.getAllByRole('textbox', { name: /^Rename group/ })).toHaveLength(40)
  fireEvent.click(screen.getByRole('button', { name: 'Next groups' }))
  expect(screen.getAllByRole('textbox', { name: /^Rename group/ })).toHaveLength(5)
  fireEvent.click(screen.getAllByRole('button', { name: /^Delete group/ })[0]!)
  expect(screen.getAllByRole('textbox', { name: /^Rename group/ })).toHaveLength(4)
  expect(onScene).not.toHaveBeenCalled()
}, 20000)

it('lets a read-only writer page presentation while disabling every mutation', () => {
  render(<Harness readOnly />)
  expect(screen.getByRole('combobox', { name: 'Arrangement mode' })).toBeDisabled()
  expect(screen.getAllByRole('textbox', { name: 'Pinned note text' })[0]).toBeDisabled()
  expect(screen.getAllByRole('button', { name: 'Delete pinned note' })[0]).toBeDisabled()
  fireEvent.click(screen.getByRole('button', { name: 'Groups & notes' }))
  expect(screen.getByRole('button', { name: 'Create group' })).toBeDisabled()
  expect(screen.getByRole('button', { name: 'Add pinned note' })).toBeDisabled()
  expect(screen.getByRole('button', { name: 'Next groups' })).toBeEnabled()
})
