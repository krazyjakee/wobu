import type { ImageAspectPreview, ImageGenerationCapabilities } from './api'

export type AspectNegotiation = ImageAspectPreview & { requestedValid: boolean }

/** Resolve saved UI state against the backend's pre-negotiated preview table. */
export function negotiatedAspect(
  capabilities: ImageGenerationCapabilities | undefined,
  requested: string,
): AspectNegotiation | undefined {
  if (!capabilities) return undefined
  const preview = capabilities.previews.find((candidate) => candidate.requestedAspect === requested)
  if (preview) return { ...preview, requestedValid: true }
  const fallback = capabilities.aspectRatios[0]
  const fallbackPreview = capabilities.previews.find(
    (candidate) => candidate.requestedAspect === fallback,
  )
  if (!fallbackPreview) return undefined
  const match = /^(\d+):(\d+)$/.exec(requested)
  const requestedValid =
    !!match &&
    Number(match[1]) > 0 &&
    Number(match[1]) <= 65_535 &&
    Number(match[2]) > 0 &&
    Number(match[2]) <= 65_535
  return { ...fallbackPreview, requestedAspect: requested, substituted: true, requestedValid }
}
