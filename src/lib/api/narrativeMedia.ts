import { call } from './call'
import { assertProjectSession, projectSessionEpoch } from '../projectSession'
import type { LocaleSource } from './narrativeLocale'
export interface MediaKey {
  id: string
  locale: string
  form: string
}
export interface MediaNotes {
  pronunciation: string
  delivery: string
}
export interface MediaPolicy {
  timing: string[]
  version: number
  required: Record<string, boolean>
  notes: Record<string, MediaNotes>
}
export interface MediaRow {
  version: number
  key: MediaKey
  source: LocaleSource
  origin: string
  translation_guard: string | null
  text: string
  notes: MediaNotes
  parameters: Record<string, string>
  media_guard: string | null
  audio_path: string
  timing_path: string | null
  audio_hash: string | null
  timing_hash: string | null
  ready: boolean
}
interface MediaBlob {
  path: string
  hash: string
  bytes: number
}
export interface MediaTake {
  row: MediaRow
  spoken_text: string
  audio: MediaBlob
  timing: MediaBlob | null
  info: { channels: number; sample_rate: number; frames: number; duration_ms: number }
  actor: string
}
export interface MediaBinding {
  version: number
  key: MediaKey
  history: MediaTake[]
}
export interface MediaDiagnostic {
  key: string
  code: string
  message: string
}
export interface MediaView {
  policy: MediaPolicy
  policy_guard: string
  locale: string
  rows: MediaRow[]
  bindings: Record<string, MediaBinding>
  diagnostics: MediaDiagnostic[]
}
export interface MediaImportReport {
  applied: string[]
  diagnostics: MediaDiagnostic[]
  conflicts: Record<string, string>
}
export interface TimingCue {
  start_ms: number
  end_ms: number
  kind: 'word' | 'phoneme' | 'viseme'
  value: string
}
export interface MediaAudition {
  path: string
  take: MediaTake
  current: boolean
  timing: { version: number; audio_hash: string; duration_ms: number; cues: TimingCue[] } | null
}
async function mediaCall<T>(command: string, args: Record<string, unknown>): Promise<T> {
  const epoch = projectSessionEpoch()
  const response = await call<T>(command, args)
  assertProjectSession(epoch)
  return response
}
export const narrativeMediaGet = (locale: string) =>
  mediaCall<MediaView>('narrative_media_get', { locale })
export const narrativeMediaPolicy = (policy: MediaPolicy, expected: string) =>
  mediaCall('narrative_media_policy', { policy, expected })
export const narrativeMediaExport = (locale: string, csv: boolean, destination: string | null) =>
  mediaCall<string>('narrative_media_export', { locale, csv, destination })
export const narrativeMediaPreview = (input: string, csv: boolean, directory: string) =>
  mediaCall<MediaDiagnostic[]>('narrative_media_preview', { input, csv, directory })
export const narrativeMediaImport = (input: string, csv: boolean, directory: string) =>
  mediaCall<MediaImportReport>('narrative_media_import', { input, csv, directory })
export const narrativeMediaAudition = (key: MediaKey, history: number) =>
  mediaCall<MediaAudition>('narrative_media_audition', { key, history })
export const mediaKey = (key: MediaKey) => `${key.locale}/${key.id}/${key.form}`
export function mediaStatus(row: MediaRow, binding: MediaBinding | undefined, invalid = false) {
  const take = binding?.history.at(-1)
  if (!take) return 'missing'
  if (invalid) return 'unavailable'
  const saved = take.row
  if (
    !row.ready ||
    JSON.stringify(saved.source) !== JSON.stringify(row.source) ||
    saved.source.guard !== row.source.guard ||
    saved.source.revision !== row.source.revision ||
    saved.translation_guard !== row.translation_guard ||
    saved.text !== row.text ||
    saved.origin !== row.origin ||
    saved.notes.pronunciation !== row.notes.pronunciation ||
    saved.notes.delivery !== row.notes.delivery
  )
    return 'out_of_date'
  return 'current'
}
