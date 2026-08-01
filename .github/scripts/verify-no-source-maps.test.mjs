import assert from 'node:assert/strict'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { afterEach, test } from 'node:test'

import { findSourceMaps, verifyNoSourceMaps } from './verify-no-source-maps.mjs'

const roots = []
afterEach(async () => Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true }))))

async function fixture() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'wobu-release-assets-test-'))
  roots.push(root)
  await mkdir(path.join(root, 'assets', 'nested'), { recursive: true })
  await writeFile(path.join(root, 'index.html'), '<main></main>')
  await writeFile(path.join(root, 'assets', 'app.js'), 'console.log("release")')
  return root
}

test('accepts release assets without source maps', async () => {
  const root = await fixture()
  await verifyNoSourceMaps(root)
})

test('reports every nested source map using stable relative paths', async () => {
  const root = await fixture()
  await writeFile(path.join(root, 'assets', 'app.js.map'), '{}')
  await writeFile(path.join(root, 'assets', 'nested', 'worker.JS.MAP'), '{}')

  assert.deepEqual(await findSourceMaps(root), ['assets/app.js.map', 'assets/nested/worker.JS.MAP'])
  await assert.rejects(
    verifyNoSourceMaps(root),
    /Release assets contain source maps:\n- assets\/app\.js\.map\n- assets\/nested\/worker\.JS\.MAP/,
  )
})

test('fails clearly when the release asset directory was not built', async () => {
  const root = path.join(await fixture(), 'missing')
  await assert.rejects(verifyNoSourceMaps(root), /Release asset directory does not exist/)
})
