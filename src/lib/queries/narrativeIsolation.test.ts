import { QueryClient } from '@tanstack/react-query'
import { expect, it } from 'vitest'
import { clearNarrativeReads, qk } from './keys'

it('drops project-specific reads, cancels late responses and retains scoped drafts', async () => {
  const qc = new QueryClient()
  const cached = [
    qk.narrativeScenes,
    qk.narrativeScene('copied-id'),
    qk.narrativeState,
    qk.narrativeLayout({ kind: 'arc', arc: 'all' }),
    ['narrative_source', '/old', 'copied-id'],
  ]
  for (const key of cached) qc.setQueryData(key, 'old project')
  const draftKey = ['narrative_source_draft', '/old', 'copied-id']
  qc.setQueryData(draftKey, 'unsaved draft')
  let resolve!: (value: string) => void
  const pending = qc
    .fetchQuery({
      queryKey: qk.narrativeScene('pending'),
      queryFn: () =>
        new Promise<string>((done) => {
          resolve = done
        }),
    })
    .catch(() => undefined)
  await clearNarrativeReads(qc)
  resolve('late old project')
  await pending
  for (const key of cached) expect(qc.getQueryData(key)).toBeUndefined()
  expect(qc.getQueryData(qk.narrativeScene('pending'))).toBeUndefined()
  expect(qc.getQueryData(draftKey)).toBe('unsaved draft')
})
