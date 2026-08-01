import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { mkdtemp, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'
import { afterEach, test } from 'node:test'

import { findSourceMaps } from './verify-no-source-maps.mjs'

const execFileAsync = promisify(execFile)
const roots = []
afterEach(async () => Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true }))))

async function build(mode) {
  const outDir = await mkdtemp(path.join(os.tmpdir(), 'wobu-vite-build-test-'))
  roots.push(outDir)

  const vite = path.resolve('node_modules/vite/bin/vite.js')
  const args = [vite, 'build', '--outDir', outDir]
  if (mode) args.push('--mode', mode)
  const envWithoutOptIn = { ...process.env }
  delete envWithoutOptIn.WOBU_BUILD_SOURCEMAPS
  await execFileAsync(process.execPath, args, { env: envWithoutOptIn })
  return outDir
}

test('ordinary production builds contain no source maps', async () => {
  const outDir = await build()
  assert.deepEqual(await findSourceMaps(outDir), [])
})

test('diagnostic builds can explicitly opt in to source maps', async () => {
  const outDir = await build('diagnostic')
  assert.ok((await findSourceMaps(outDir)).length > 0)
})
