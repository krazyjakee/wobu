export type EditorWriteState = 'clean' | 'pending' | 'saving' | 'failed'

export interface EditorWriteView {
  nodeId: string | null
  state: EditorWriteState
  error: unknown
}

interface EditorWriter extends EditorWriteView {
  flushAndSettle: () => Promise<void>
}

export interface EditorWriterHandle {
  update: (view: EditorWriteView) => void
  unregister: () => void
}

/**
 * The close paths cannot depend on whichever editor happens to be mounted.
 * This registry is the process-wide view of dirty, in-flight, and failed editor
 * writes; each autosave hook contributes one writer for as long as it is live.
 */
class EditorWriteCoordinator {
  private sequence = 0
  private readonly writers = new Map<number, EditorWriter>()

  register(writer: EditorWriter): EditorWriterHandle {
    const id = ++this.sequence
    this.writers.set(id, writer)
    return {
      update: (view) => {
        const current = this.writers.get(id)
        if (current) Object.assign(current, view)
      },
      unregister: () => this.writers.delete(id),
    }
  }

  snapshot(): EditorWriteView[] {
    return [...this.writers.values()].map(({ nodeId, state, error }) => ({
      nodeId,
      state,
      error,
    }))
  }

  async flushAll(): Promise<void> {
    const writers = [...this.writers.values()]
    const results = await Promise.allSettled(writers.map((writer) => writer.flushAndSettle()))
    const failed = results.filter((result) => result.status === 'rejected')
    const unsettled = this.snapshot().filter((write) => write.state !== 'clean')
    if (failed.length > 0 || unsettled.length > 0) {
      throw new EditorWritesBlocked(unsettled, failed[0]?.reason)
    }
  }

  /** Test isolation for this process-wide registry. */
  reset(): void {
    this.writers.clear()
  }
}

export class EditorWritesBlocked extends Error {
  constructor(
    readonly writes: EditorWriteView[],
    options?: unknown,
  ) {
    super('Editor writes did not settle; the project must remain open', { cause: options })
    this.name = 'EditorWritesBlocked'
  }
}

export const editorWrites = new EditorWriteCoordinator()

export function isEditorWritesBlocked(error: unknown): error is EditorWritesBlocked {
  return error instanceof EditorWritesBlocked
}
