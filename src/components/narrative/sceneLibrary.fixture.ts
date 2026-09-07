import type { Scene } from '../../lib/api'
import type { LibraryRow } from './sceneLibraryModel'

export const libraryScene: Scene = {
  id: 'council',
  name: 'Council hearing',
  participants: [{ entity: 'mira' }],
  beats: [
    {
      id: 'evidence',
      title: 'Evidence',
      intents: [{ subject: 'player', intent: 'Convince the council' }],
      dialogue: [
        {
          id: 'line',
          policy: 'edited',
          speaker: { entity: 'mira' },
          variants: [
            {
              id: 'high',
              text: {
                revision: 'a',
                body: 'I saw the attack myself.',
                lifecycle: { policy: 'locked', review: 'approved', freshness: 'out_of_date' },
              },
            },
            { id: 'low', text: { revision: 'b', body: 'They told me about the attack.' } },
            {
              id: 'draft',
              text: {
                revision: 'c',
                body: 'A secret generated proposal.',
                provenance: { generated: { fingerprint: 'x' } },
              },
            },
          ],
        },
      ],
    },
  ],
}
export const libraryRows: LibraryRow[] = [
  {
    summary: { id: 'council', name: 'Council hearing', slug: 'council', rel: 'council.yaml' },
    scene: libraryScene,
  },
]
