import { call } from './call'
/* ── domain types ─────────────────────────────────────────────────────────── */

/** The local SQLite index for the open project. Disposable by design. */
export interface IndexInfo {
  path: string
  sizeBytes: number
  nodeCount: number
}

export const indexInfo = () => call<IndexInfo>('index_info')

/** Throw the index away and rebuild it from the Markdown. */
export const indexRebuild = () => call<void>('index_rebuild')

export interface AboutInfo {
  appVersion: string
  /** The on-disk format of the project folder. */
  projectSchemaVersion: number
  /** The local index layout; a bump silently rebuilds on next open. */
  indexSchemaVersion: number
  logPath: string
}

export const aboutInfo = () => call<AboutInfo>('about_info')

/* ── diagnostics ──────────────────────────────────────────────────────────── */

/**
 * Least to most verbose. `off` records nothing at all, errors included — it is
 * there for someone who wants the file to stop existing, not as a default.
 */
export type LogLevel = 'off' | 'error' | 'warn' | 'info' | 'debug'

export const LOG_LEVELS: LogLevel[] = ['off', 'error', 'warn', 'info', 'debug']

export interface LogInfo {
  /** Absolute. Shown to the user, who may well go and find it by hand. */
  path: string
  level: LogLevel
  /** False until something has been recorded — there may be nothing to reveal. */
  exists: boolean
  sizeBytes: number
}

export const logInfo = () => call<LogInfo>('log_info')

export const logSetLevel = (level: LogLevel) => call<void>('log_set_level', { level })

/** The end of the log, so the user can read it before handing it over. */
export const logTail = (lines: number) => call<string>('log_tail', { lines })

/** Show it in the OS file manager, which is how it gets attached to something. */
export const logReveal = () => call<void>('log_reveal')

/* ── jobs ─────────────────────────────────────────────────────────────────── */
