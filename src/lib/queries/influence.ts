import { keepPreviousData, useQuery, type UseQueryResult } from '@tanstack/react-query'
import * as api from '../api'
import type {
  CompiledPrompt,
  InfluenceStack,
  PromptBudget,
  ShotControls,
  SliderSetting,
} from '../api'
import { qk } from './keys'
/* ── keys ─────────────────────────────────────────────────────────────────── */

/** What `influenceResolve` varies on, and therefore what goes in its key. */
export interface InfluenceOptions {
  preset?: string
  sliders?: SliderSetting[]
  shot?: ShotControls
}

export interface PromptOptions extends InfluenceOptions {
  budget?: PromptBudget
}

/**
 * The resolved stack for a subject — one card per layer, outermost first.
 *
 * `staleTime: Infinity` because the answer is a pure function of the world and
 * the arguments: nothing but a world change can move it, and that arrives as
 * `world:changed` and invalidates this by hand. Refetching on window focus would
 * be a round trip guaranteed to return what is already on screen.
 */
export function useInfluenceStack(
  subjectId: string | null,
  options: InfluenceOptions = {},
): UseQueryResult<InfluenceStack> {
  return useQuery({
    queryKey: qk.influence(subjectId ?? '', options),
    queryFn: () => api.influenceResolve(subjectId as string, options),
    enabled: !!subjectId,
    staleTime: Infinity,
    retry: false,
  })
}

/**
 * The compiled prompt, its spans, and the account of what was dropped.
 *
 * `keepPreviousData` is what makes this usable while a slider is moving: every
 * value is a new key, so without it the prompt box would empty and refill under
 * the cursor on every frame of a drag. The backend does no file I/O for this, so
 * running it per drag is cheap — it is the blanking that would be unacceptable,
 * not the call.
 *
 * `gcTime` is short for the same reason. A single drag leaves one cache entry
 * per position it passed through, and none of them will ever be asked for again.
 */
export function useCompiledPrompt(
  subjectId: string | null,
  options: PromptOptions = {},
): UseQueryResult<CompiledPrompt> {
  return useQuery({
    queryKey: qk.prompt(subjectId ?? '', options),
    queryFn: () => api.promptCompile(subjectId as string, options),
    enabled: !!subjectId,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    gcTime: 30_000,
    retry: false,
  })
}

export function useImageReferenceReport(
  subjectId: string | null,
  options: Pick<
    api.GenerateOptions,
    'preset' | 'sliders' | 'shot' | 'aspect' | 'model' | 'seed' | 'grid'
  > = {},
): UseQueryResult<api.ImageReferenceReport> {
  return useQuery({
    queryKey: qk.imageReferences(subjectId ?? '', options),
    queryFn: () => api.imageReferenceReport(subjectId as string, options),
    enabled: !!subjectId,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
    gcTime: 30_000,
    retry: false,
  })
}

/** Provider-owned aspect choices and pre-queue negotiation for generation UIs. */
export function useImageGenerationCapabilities(
  project: string,
  model?: string,
  enabled = true,
): UseQueryResult<api.ImageGenerationCapabilities> {
  return useQuery({
    queryKey: qk.imageGenerationCapabilities(project, model),
    queryFn: () => api.imageGenerationCapabilities(model),
    enabled,
    retry: false,
  })
}
