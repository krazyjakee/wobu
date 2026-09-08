import { useQuery } from '@tanstack/react-query'
import {
  narrativeTextDiagnostics,
  narrativeTextGet,
  narrativeTexts,
  type TextAsset,
  type TextAssetId,
} from '../api/narrativeText'

/**
 * Reads for the Text library (#167).
 *
 * Two families rather than one — `narrative_texts` for the catalog and
 * `narrative_text` for a document — matching the scene split, so saving one
 * asset does not throw away the list and reopening the list does not re-read
 * every file. Both are registered in `keys.ts` so that opening a second project
 * cannot render the first project's barks.
 *
 * `retry: false` throughout: these are local filesystem reads, and a failure is
 * a broken file or a disconnected share rather than a flaky network. Retrying
 * would only delay showing the writer what is wrong.
 */
export function useNarrativeTexts(projectKey: string, enabled = true) {
  return useQuery({
    queryKey: ['narrative_texts', projectKey],
    queryFn: narrativeTexts,
    enabled,
    retry: false,
  })
}

export function useNarrativeText(projectKey: string, assetId: TextAssetId | null) {
  return useQuery({
    queryKey: ['narrative_text', projectKey, assetId],
    queryFn: () => narrativeTextGet(assetId as TextAssetId),
    enabled: Boolean(assetId),
    retry: false,
  })
}

/**
 * What is wrong with the document as the editor currently holds it.
 *
 * `asset` is part of the key rather than a bare argument, because the whole
 * point is that unsaved edits change the answer: a key that ignored the draft
 * would show the writer problems they fixed a minute ago.
 */
export function useNarrativeTextDiagnostics(
  projectKey: string,
  assetId: TextAssetId | null,
  asset: TextAsset | null,
) {
  return useQuery({
    queryKey: ['narrative_diagnostics', 'text', projectKey, assetId, asset],
    queryFn: () => narrativeTextDiagnostics(assetId as TextAssetId, asset),
    enabled: Boolean(assetId),
    retry: false,
  })
}
