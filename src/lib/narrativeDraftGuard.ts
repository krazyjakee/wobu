import { editorWrites, type EditorWriterHandle } from './editorWrites'

// Draft buffers outlive their tabs. Keep their close guards alive too, until
// an explicit save or discard, so switching to Flow cannot make quit lose text.
const guards = new Map<string, EditorWriterHandle>()

export function resetNarrativeDraftGuards(): void {
  for (const handle of guards.values()) handle.unregister()
  guards.clear()
}

export function setNarrativeDraftGuard(key: string, dirty: boolean): void {
  if (!dirty) {
    guards.get(key)?.unregister()
    guards.delete(key)
  } else if (!guards.has(key)) {
    guards.set(
      key,
      editorWrites.register({
        nodeId: null,
        state: 'pending',
        error: null,
        flushAndSettle: async () => {
          throw new Error('Save or discard the narrative draft before closing the project.')
        },
      }),
    )
  }
}
