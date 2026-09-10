/** Parse integer spelling before Number can round a decimal into a different integer. */
export function parseSafeInteger(value: string): number | undefined {
  if (!/^[+-]?\d+$/.test(value)) return undefined
  const parsed = Number(value)
  return Number.isSafeInteger(parsed) ? parsed : undefined
}

/** Refuse writes of loaded i64 values that the webview cannot represent exactly. */
export function hasUnsafeInteger(value: unknown): boolean {
  if (typeof value === 'number') return !Number.isSafeInteger(value)
  return !!value && typeof value === 'object' && Object.values(value).some(hasUnsafeInteger)
}
