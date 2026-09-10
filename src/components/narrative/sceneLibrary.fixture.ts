import type { LibraryPage, LibrarySceneRow } from '../../lib/api/narrativeLibrary'
export const libraryRow: LibrarySceneRow = {
  summary: {
    id: 'council',
    name: 'Council hearing',
    slug: 'council',
    rel: 'narrative/scenes/council.yaml',
  },
  act: { id: 'arrival', name: 'Arrival' },
  arc: { id: 'inquiry', name: 'Inquiry' },
  setting: { id: 'chamber', name: 'Council chamber' },
  tags: [{ id: 'politics', name: 'Politics' }],
  quests: [
    { id: 'attack', name: 'Investigate the attack' },
    { id: 'trust', name: 'Earn council trust' },
  ],
  participants: ['mira'],
  slots: 50,
  filled: 49,
  beats: 4,
  counts: { generated: 10, edited: 20, locked: 20, needsReview: 20, outOfDate: 3 },
  matches: [
    {
      sceneId: 'council',
      beatId: 'evidence',
      lineId: 'line',
      variantId: 'high',
      snippet: 'We saw the attack.',
      draft: false,
    },
    {
      sceneId: 'council',
      beatId: 'evidence',
      lineId: 'line',
      variantId: 'low',
      snippet: 'The attack changed everything.',
      draft: false,
    },
  ],
  matchCount: 2,
}
export const libraryPage: LibraryPage = {
  revision: 'a'.repeat(64),
  total: 1,
  sceneCount: 1,
  rows: [libraryRow],
  unreadable: [],
  unreadableTotal: 0,
  unreadableNextOffset: null,
  missingIds: [],
  facets: {
    participants: ['mira'],
    acts: [libraryRow.act!],
    arcs: [libraryRow.arc!],
    settings: [libraryRow.setting!],
    tags: libraryRow.tags,
    quests: libraryRow.quests,
  },
}

import type { Scene } from '../../lib/api'
import type { SceneSummary } from '../../lib/api'
type LibraryRow = { summary: SceneSummary; scene?: Scene }

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
