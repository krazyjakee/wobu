import type { SceneFile } from '../../lib/api'
export const sessionFile = (): SceneFile => ({
  slug: 'scene',
  rel: 'narrative/scenes/scene.yaml',
  stamp: { hash: 'base', size: 1, mtime_ms: 1 },
  scene: {
    id: 'scene',
    name: 'Ashfall',
    beats: [
      {
        id: 'beat',
        title: 'Evidence',
        dialogue: [
          {
            id: 'slot',
            speaker: 'player',
            variants: [{ id: 'variant', text: { body: 'First wording', revision: 'first' } }],
          },
        ],
      },
    ],
  },
})
