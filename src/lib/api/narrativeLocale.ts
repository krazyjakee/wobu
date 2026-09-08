import { call } from './call'
import { assertProjectSession, projectSessionEpoch } from '../projectSession'
async function sessionCall<T>(command: string, args: Record<string, unknown>): Promise<T> {
  const epoch = projectSessionEpoch()
  const result = await call<T>(command, args)
  assertProjectSession(epoch)
  return result
}
export interface LocaleSource {
  id: string
  slot: string
  container: string
  speaker: string
  text: string
  revision: string
  guard: string
  context: string
  delivery_notes: string
  placeholders: string[]
  ready: boolean
}
export interface LocalePolicy {
  version: number
  source: string
  required: Record<string, boolean>
}
export interface TranslationVersion {
  source_revision: string
  source_guard: string
  forms: Record<string, string>
  approved: boolean
  actor: string
}
export interface Translation {
  version: number
  variant_id: string
  locale: string
  history: TranslationVersion[]
}
export interface LocaleView {
  policy: LocalePolicy
  policy_guard: string
  sources: Record<string, LocaleSource>
  translations: Translation[]
  translation_guards: Record<string, string>
}
export interface LocaleRow {
  version: number
  locale: string
  source: LocaleSource
  translation_guard: string | null
  forms: Record<string, string>
}
export interface LocaleDiagnostic {
  id: string
  code: string
  message: string
}
export interface LocaleImportReport {
  applied: string[]
  diagnostics: LocaleDiagnostic[]
  conflicts: Record<string, string>
}
export const narrativeLocaleGet = () => sessionCall<LocaleView>('narrative_locale_get', {})
export const narrativeLocalePolicy = (policy: LocalePolicy, expected: string) =>
  sessionCall('narrative_locale_policy', { policy, expected })
export const narrativeLocaleExport = (locale: string, csv: boolean, destination: string | null) =>
  sessionCall<string>('narrative_locale_export', { locale, csv, destination })
export const narrativeLocalePreview = (input: string, csv: boolean) =>
  sessionCall<LocaleDiagnostic[]>('narrative_locale_preview', { input, csv })
export const narrativeLocaleImport = (input: string, csv: boolean) =>
  sessionCall<LocaleImportReport>('narrative_locale_import', { input, csv })
export const narrativeLocaleApprove = (row: LocaleRow) =>
  sessionCall('narrative_locale_approve', { row })
export function translationIndex(view: LocaleView | undefined) {
  return new Map(view?.translations.map((t) => [`${t.locale}/${t.variant_id}`, t]))
}
export function translationStatus(
  view: LocaleView,
  locale: string,
  id: string,
  index = translationIndex(view),
) {
  const source = view.sources[id]
  const translation = index.get(`${locale}/${id}`)
  const latest = translation?.history.at(-1)
  if (!latest) return 'missing'
  if (
    !source?.ready ||
    latest.source_guard !== source.guard ||
    latest.source_revision !== source.revision
  )
    return 'out_of_date'
  return latest.approved ? 'approved' : 'draft'
}

/** Display the backend's canonical case for admitted language/script/region tags. */
export function canonicalLocale(value: string) {
  return value
    .trim()
    .split(/[-_]/)
    .map((part, index) =>
      index === 0
        ? part.toLowerCase()
        : part.length === 4
          ? part[0]!.toUpperCase() + part.slice(1).toLowerCase()
          : part.toUpperCase(),
    )
    .join('-')
}
