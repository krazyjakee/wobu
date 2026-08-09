import assert from 'node:assert/strict'
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { afterEach, test } from 'node:test'

import { checkRelease, inspectRelease, stampAppVersion } from './release-version.mjs'

const roots = []
afterEach(async () => Promise.all(roots.splice(0).map((root) => rm(root, { recursive: true }))))

async function fixture(overrides = {}) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'wobu-release-test-'))
  roots.push(root)
  for (const dir of [
    'release',
    'src-tauri/crates/wobu-core/src',
    'src-tauri/crates/wobu-store/src/index',
  ]) {
    await mkdir(path.join(root, dir), { recursive: true })
  }
  const version = overrides.version ?? '0.1.0'
  const packageVersion = overrides.packageVersion ?? version
  await writeFile(
    path.join(root, 'package.json'),
    `${JSON.stringify({ name: 'wobu', version: packageVersion }, null, 2)}\n`,
  )
  await writeFile(
    path.join(root, 'package-lock.json'),
    `${JSON.stringify({ name: 'wobu', version, packages: { '': { name: 'wobu', version } } }, null, 2)}\n`,
  )
  await writeFile(
    path.join(root, 'src-tauri/tauri.conf.json'),
    `${JSON.stringify(
      {
        version,
        plugins: {
          updater: { endpoints: ['https://example.invalid/latest.json'], pubkey: 'dGVzdA==' },
        },
        bundle: { active: true, targets: 'all', createUpdaterArtifacts: true, icon: [] },
      },
      null,
      2,
    )}\n`,
  )
  await writeFile(
    path.join(root, 'release/versions.json'),
    `${JSON.stringify({ appVersion: version, projectSchemaVersion: 1, indexSchemaVersion: 9 }, null, 2)}\n`,
  )
  await writeFile(
    path.join(root, 'src-tauri/Cargo.toml'),
    `[workspace]\nmembers = ["crates/*"]\n\n[workspace.package]\nversion = "${version}"\n`,
  )
  await writeFile(
    path.join(root, 'src-tauri/Cargo.lock'),
    `[[package]]\nname = "not-wobu"\nversion = "0.1.0"\n\n[[package]]\nname = "wobu"\nversion = "${version}"\n\n[[package]]\nname = "wobu-core"\nversion = "${version}"\n`,
  )
  await writeFile(
    path.join(root, 'src-tauri/crates/wobu-core/src/lib.rs'),
    'pub const SCHEMA_VERSION: u32 = 1;\n',
  )
  await writeFile(
    path.join(root, 'src-tauri/crates/wobu-store/src/index/mod.rs'),
    'pub const INDEX_VERSION: u32 = 9;\n',
  )
  return root
}

test('check names every drifted app stamp and keeps schemas separate', async () => {
  const root = await fixture({ packageVersion: '0.1.1' })
  const result = await checkRelease(root)
  assert.deepEqual(result.errors, ['packageJson is 0.1.1, expected 0.1.0'])
  assert.equal(result.state.projectSchemaVersion, 1)
  assert.equal(result.state.indexSchemaVersion, 9)
})

test('stamping updates every app package but never schema versions or unrelated crates', async () => {
  const root = await fixture()
  const tauriFile = path.join(root, 'src-tauri/tauri.conf.json')
  const tauriBefore = await readFile(tauriFile, 'utf8')
  await stampAppVersion(root, '1.2.3-beta.1')
  const { errors } = await checkRelease(root)
  assert.deepEqual(errors, [])

  const state = await inspectRelease(root)
  assert.equal(state.declared.appVersion, '1.2.3-beta.1')
  assert.equal(state.projectSchemaVersion, 1)
  assert.equal(state.indexSchemaVersion, 9)
  const lock = await readFile(path.join(root, 'src-tauri/Cargo.lock'), 'utf8')
  assert.match(lock, /name = "not-wobu"\nversion = "0\.1\.0"/)
  assert.match(lock, /name = "wobu-core"\nversion = "1\.2\.3-beta\.1"/)
  const tauriAfter = await readFile(tauriFile, 'utf8')
  assert.equal(
    tauriAfter.replace('1.2.3-beta.1', '0.1.0'),
    tauriBefore,
    'stamping the version must not reformat unrelated Tauri configuration',
  )
})

test('in-place updates require updater artifacts to stay enabled', async () => {
  const root = await fixture()
  const configFile = path.join(root, 'src-tauri/tauri.conf.json')
  const config = JSON.parse(await readFile(configFile, 'utf8'))
  config.bundle.createUpdaterArtifacts = false
  await writeFile(configFile, `${JSON.stringify(config, null, 2)}\n`)

  const { errors } = await checkRelease(root)
  assert.ok(errors.some((error) => error.includes('updater artifacts')))
})

// A client with no key verifies nothing, and one reached over plain HTTP is a
// payload an intermediary chooses. Both are quiet: the app still builds, still
// finds an update, and still installs it.
test('an updater without a public key or with an insecure endpoint is refused', async () => {
  const root = await fixture()
  const configFile = path.join(root, 'src-tauri/tauri.conf.json')
  const config = JSON.parse(await readFile(configFile, 'utf8'))
  config.plugins.updater = { endpoints: ['http://example.invalid/latest.json'], pubkey: '  ' }
  await writeFile(configFile, `${JSON.stringify(config, null, 2)}\n`)

  const { errors } = await checkRelease(root)
  assert.ok(errors.some((error) => error.includes('must be HTTPS')))
  assert.ok(errors.some((error) => error.includes('public key')))
})

test('an updater with no endpoint at all is refused', async () => {
  const root = await fixture()
  const configFile = path.join(root, 'src-tauri/tauri.conf.json')
  const config = JSON.parse(await readFile(configFile, 'utf8'))
  delete config.plugins
  await writeFile(configFile, `${JSON.stringify(config, null, 2)}\n`)

  const { errors } = await checkRelease(root)
  assert.ok(errors.some((error) => error.includes('no endpoint')))
})

test('a publication failure restores every prior version instead of leaving mixed stamps', async () => {
  const root = await fixture()
  await assert.rejects(
    stampAppVersion(root, '2.0.0', {
      beforeReplace: ({ index }) => {
        if (index === 1) throw new Error('simulated second-file failure')
      },
    }),
    /simulated second-file failure/,
  )

  const { errors, state } = await checkRelease(root)
  assert.deepEqual(errors, [])
  assert.equal(state.declared.appVersion, '0.1.0')
  assert.equal(state.appVersions.packageJson, '0.1.0')
})

test('stamping refuses before publication when Cargo.lock has no Wobu package', async () => {
  const root = await fixture()
  const lockFile = path.join(root, 'src-tauri/Cargo.lock')
  await writeFile(lockFile, '[[package]]\nname = "unrelated"\nversion = "0.1.0"\n')

  await assert.rejects(stampAppVersion(root, '3.0.0'), /no Wobu workspace package/)
  assert.equal(JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8')).version, '0.1.0')
  assert.equal(
    JSON.parse(await readFile(path.join(root, 'release/versions.json'), 'utf8')).appVersion,
    '0.1.0',
  )
})

test('numeric prerelease identifiers with leading zero are not SemVer', async () => {
  const root = await fixture()
  await assert.rejects(stampAppVersion(root, '1.2.3-01'), /Not a SemVer/)
  assert.equal(JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8')).version, '0.1.0')
})
