import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, it } from 'vitest'
import type { WorldFile } from '../../lib/api/narrativeWorld'
import { qk } from '../../lib/queries/keys'
import { useUI } from '../../store/ui'
import { NarrativeBacklinks } from './NarrativeBacklinks'
import { NarrativeWorldPane } from './NarrativeWorldPane'
import { narrativeBacklinks } from './narrativeBacklinkModel'
import { useWorldDrafts } from './worldDrafts'
const world = (): WorldFile => ({
  document: {
    schema_version: 2,
    facts: [
      {
        id: 'beacon',
        name: 'The fallen beacon',
        assertion: 'The beacon fell at Cinder Bay.',
        sources: [],
        entity_ids: ['bay'],
      },
    ],
    knowledge: [
      {
        id: 'witness',
        name: 'Kael witnessed the fall',
        character: 'kael',
        fact: 'beacon',
        belief: 'true',
        provenance: 'witnessed',
        when: 'always',
      },
    ],
    relationships: [],
    events: [],
    quests: [],
    restrictions: [],
  },
  stamp: null,
  diagnostics: [],
})
beforeEach(() => {
  useUI.setState({
    narrativeWorldTarget: null,
    narrativeEntityTarget: null,
    selectedId: null,
    recentIds: ['bay'],
    tab: 'refs',
    mode: 'library',
  })
  useWorldDrafts.setState({ world: {}, variables: {} })
})
it('links by typed entity identity and keeps the same record target after renaming', () => {
  const document = world().document
  expect(narrativeBacklinks(document, 'bay', false).map((link) => link.record.id)).toEqual([
    'beacon',
  ])
  expect(narrativeBacklinks(document, 'kael', true).map((link) => link.record.id)).toEqual([
    'witness',
  ])
  expect(narrativeBacklinks(document, 'Cinder Bay', false)).toEqual([])
  document.knowledge[0]!.name = 'Renamed testimony'
  expect(narrativeBacklinks(document, 'kael', true)[0]!.record.id).toBe('witness')
})
it('opens a character backlink at its exact World record and returns to the entity detail', async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { staleTime: Infinity, retry: false } },
  })
  client.setQueryData(['narrative_world'], world())
  client.setQueryData(qk.nodes, [
    { id: 'kael', name: 'Kael Vantris', kind: 'character' },
    { id: 'bay', name: 'Cinder Bay', kind: 'setting' },
  ])
  client.setQueryData(qk.narrativeScenes, { scenes: [], unreadable: [] })
  client.setQueryData(qk.narrativeState, {
    document: { schema_version: 1, variables: [] },
    stamp: null,
  })
  render(
    <QueryClientProvider client={client}>
      <NarrativeBacklinks projectKey="/project" entityId="kael" character />
      <NarrativeWorldPane projectKey="/project" readOnly={false} />
    </QueryClientProvider>,
  )
  fireEvent.click(screen.getByText('Narrative records (1)'))
  fireEvent.click(screen.getByRole('button', { name: 'Kael witnessed the fall · Knowledge' }))
  expect(useUI.getState().narrativeWorldTarget).toMatchObject({
    projectKey: '/project',
    collection: 'knowledge',
    recordId: 'witness',
  })
  expect(useUI.getState().mode).toBe('narrative')
  expect(screen.getByLabelText('Record name')).toHaveValue('Kael witnessed the fall')
  const renamed = world()
  renamed.document.knowledge[0]!.name = 'Renamed testimony'
  act(() => {
    client.setQueryData(['narrative_world'], renamed)
  })
  expect(
    await screen.findByRole('button', { name: 'Renamed testimony · Knowledge' }),
  ).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Kael Vantris' }))
  expect(useUI.getState().selectedId).toBe('kael')
  expect(useUI.getState().mode).toBe('library')
  expect(screen.getByText('Narrative records (1)')).toHaveFocus()
  expect(useUI.getState().narrativeEntityTarget).toBeNull()
  expect(useUI.getState().recentIds).toEqual(['kael', 'bay'])
  expect(useUI.getState().tab).toBe('refs')
  fireEvent.click(screen.getByRole('button', { name: 'Renamed testimony · Knowledge' }))
  expect(screen.getByLabelText('Record name')).toHaveFocus()
  fireEvent.click(screen.getByRole('button', { name: 'Kael Vantris' }))
  expect(screen.getByText('Narrative records (1)')).toHaveFocus()
  expect(useUI.getState().recentIds).toEqual(['kael', 'bay'])
})

it('announces a deleted backlink target and focuses the surviving category record', () => {
  const client = new QueryClient({
    defaultOptions: { queries: { staleTime: Infinity, retry: false } },
  })
  client.setQueryData(['narrative_world'], world())
  client.setQueryData(qk.nodes, [])
  client.setQueryData(qk.narrativeScenes, { scenes: [], unreadable: [] })
  client.setQueryData(qk.narrativeState, {
    document: { schema_version: 1, variables: [] },
    stamp: null,
  })
  useUI
    .getState()
    .openNarrativeWorld({ projectKey: '/project', collection: 'facts', recordId: 'deleted' })
  render(
    <QueryClientProvider client={client}>
      <NarrativeWorldPane projectKey="/project" readOnly={false} />
    </QueryClientProvider>,
  )
  expect(screen.getByRole('status')).toHaveTextContent('The requested record no longer exists')
  expect(screen.getByLabelText('Record name')).toHaveValue('The fallen beacon')
  expect(screen.getByLabelText('Record name')).toHaveFocus()
})
