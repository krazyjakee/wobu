import { useState } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { NarrativeGeneration } from '../NarrativeGeneration'
import { NarrativeReview } from '../NarrativeReview'
import type { reviewLoopFixture } from './reviewLoopFixture.test-support'
import { qk } from '../../../lib/queries/keys'
export function ReviewLoop({ fixture }: { fixture: ReturnType<typeof reviewLoopFixture> }) {
  const [mode, setMode] = useState<'generation' | 'review' | null>(null)
  const [client] = useState(() => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } })
    client.setQueryData(qk.projectCurrent, { id: 'review-loop', path: '/review-loop' })
    return client
  })
  return (
    <QueryClientProvider client={client}>
      <main>
        <h1>Beacon aftermath — scripted review loop</h1>
        <p>
          Actual React components. Mocked IPC and provider results; no native backend or live
          provider.
        </p>
        <button className="btn" onClick={() => setMode('generation')}>
          Open generation
        </button>
        <button className="btn" onClick={() => setMode('review')}>
          Open review
        </button>
        <button className="btn" onClick={() => fixture.changeContext()}>
          Simulate upstream context change
        </button>
        {mode === 'generation' && (
          <NarrativeGeneration
            scene={fixture.scene()}
            projectKey="/review-loop"
            readOnly={false}
            onClose={() => setMode(null)}
          />
        )}
        {mode === 'review' && (
          <NarrativeReview
            projectKey="/review-loop"
            readOnly={false}
            sceneName={() => 'Beacon aftermath'}
            speakerName={() => 'Mara Voss'}
            onSource={() => {}}
            onClose={() => setMode(null)}
          />
        )}
      </main>
    </QueryClientProvider>
  )
}
