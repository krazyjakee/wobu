import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const config = JSON.parse(readFileSync(join(root, '.jscpd.json'), 'utf8'))
const baseline = JSON.parse(readFileSync(join(root, '.jscpd-baseline.json'), 'utf8'))

if (
  baseline.detector.minLines !== config.minLines ||
  baseline.detector.minTokens !== config.minTokens
) {
  console.error('The jscpd detector settings and committed baseline disagree.')
  process.exit(2)
}

const output = mkdtempSync(join(tmpdir(), 'wobu-jscpd-'))

try {
  const result = spawnSync(
    process.execPath,
    [
      join(root, 'node_modules/jscpd/run-jscpd.js'),
      'src',
      'src-tauri',
      '--reporters',
      'json',
      '--output',
      output,
    ],
    { cwd: root, encoding: 'utf8' },
  )

  if (result.status !== 0) {
    process.stdout.write(result.stdout)
    process.stderr.write(result.stderr)
    process.exit(result.status ?? 2)
  }

  const report = JSON.parse(readFileSync(join(output, 'jscpd-report.json'), 'utf8'))
  const current = report.statistics.formats
  const formats = [...new Set([...Object.keys(baseline.formats), ...Object.keys(current)])].sort()
  const regressions = []

  for (const format of formats) {
    const actual = current[format] ?? { clones: 0, duplicatedLines: 0 }
    const allowed = baseline.formats[format] ?? { clones: 0, duplicatedLines: 0 }
    const summary = `${format}: ${actual.clones}/${allowed.clones} clone blocks, ${actual.duplicatedLines}/${allowed.duplicatedLines} duplicated lines`

    if (actual.clones > allowed.clones || actual.duplicatedLines > allowed.duplicatedLines) {
      regressions.push(summary)
      console.error(`REGRESSION ${summary}`)
    } else {
      console.log(`OK ${summary}`)
    }
  }

  if (regressions.length > 0) {
    console.error(
      `Duplication exceeds the committed baseline at ${config.minLines} lines / ${config.minTokens} tokens.`,
    )
    process.exit(1)
  }
} finally {
  rmSync(output, { recursive: true, force: true })
}
