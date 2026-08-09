#!/usr/bin/env node

import { chmod, readFile, rename, rm, stat, writeFile } from 'node:fs/promises'
import { randomUUID } from 'node:crypto'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/
const WORKSPACE_PACKAGE = /^wobu(?:-[a-z0-9-]+)?$/

const readJson = async (file) => JSON.parse(await readFile(file, 'utf8'))
const json = (value) => `${JSON.stringify(value, null, 2)}\n`

function isSemVer(value) {
  const match = typeof value === 'string' ? value.match(SEMVER) : null
  if (!match) return false
  return (match[4]?.split('.') ?? []).every(
    (identifier) => !/^\d+$/.test(identifier) || identifier === '0' || !identifier.startsWith('0'),
  )
}

function one(text, pattern, label) {
  const match = text.match(pattern)
  if (!match) throw new Error(`Could not read ${label}`)
  return match[1]
}

export async function inspectRelease(root) {
  const packageJson = await readJson(path.join(root, 'package.json'))
  const packageLock = await readJson(path.join(root, 'package-lock.json'))
  const tauri = await readJson(path.join(root, 'src-tauri/tauri.conf.json'))
  const declared = await readJson(path.join(root, 'release/versions.json'))
  const cargoToml = await readFile(path.join(root, 'src-tauri/Cargo.toml'), 'utf8')
  const cargoLock = await readFile(path.join(root, 'src-tauri/Cargo.lock'), 'utf8')
  const core = await readFile(path.join(root, 'src-tauri/crates/wobu-core/src/lib.rs'), 'utf8')
  const index = await readFile(
    path.join(root, 'src-tauri/crates/wobu-store/src/index/mod.rs'),
    'utf8',
  )

  const cargoVersion = one(
    cargoToml,
    /\[workspace\.package\][\s\S]*?\nversion = "([^"]+)"/,
    'workspace package version',
  )
  const lockVersions = []
  for (const match of cargoLock.matchAll(
    /\[\[package\]\]\r?\nname = "([^"]+)"\r?\nversion = "([^"]+)"/g,
  )) {
    if (WORKSPACE_PACKAGE.test(match[1])) lockVersions.push([match[1], match[2]])
  }

  return {
    declared,
    appVersions: {
      releaseManifest: declared.appVersion,
      packageJson: packageJson.version,
      packageLock: packageLock.version,
      packageLockRoot: packageLock.packages?.['']?.version,
      tauri: tauri.version,
      cargoWorkspace: cargoVersion,
    },
    lockVersions,
    projectSchemaVersion: Number(
      one(core, /pub const SCHEMA_VERSION: u32 = (\d+);/, 'project schema version'),
    ),
    indexSchemaVersion: Number(
      one(index, /pub const INDEX_VERSION: u32 = (\d+);/, 'index schema version'),
    ),
    bundle: tauri.bundle,
    updater: tauri.plugins?.updater,
    iconPaths: (tauri.bundle?.icon ?? []).map((icon) => path.join(root, 'src-tauri', icon)),
  }
}

export async function checkRelease(root) {
  const state = await inspectRelease(root)
  const errors = []
  if (!isSemVer(state.declared.appVersion)) {
    errors.push(`release appVersion is not SemVer: ${state.declared.appVersion}`)
  }
  for (const [source, version] of Object.entries(state.appVersions)) {
    if (version !== state.declared.appVersion) {
      errors.push(`${source} is ${String(version)}, expected ${state.declared.appVersion}`)
    }
  }
  if (state.lockVersions.length === 0) errors.push('Cargo.lock has no Wobu workspace packages')
  for (const [name, version] of state.lockVersions) {
    if (version !== state.declared.appVersion) {
      errors.push(`Cargo.lock ${name} is ${version}, expected ${state.declared.appVersion}`)
    }
  }
  if (state.projectSchemaVersion !== state.declared.projectSchemaVersion) {
    errors.push(
      `project schema is ${state.projectSchemaVersion}, release manifest expects ${state.declared.projectSchemaVersion}`,
    )
  }
  if (state.indexSchemaVersion !== state.declared.indexSchemaVersion) {
    errors.push(
      `index schema is ${state.indexSchemaVersion}, release manifest expects ${state.declared.indexSchemaVersion}`,
    )
  }
  if (state.bundle?.active !== true) errors.push('Tauri bundle.active must be true')
  if (state.bundle?.targets !== 'all') errors.push('Tauri bundle.targets must be "all"')
  // The three halves of a working update, checked together because any one of
  // them alone ships a client that either cannot be updated or — worse — has
  // nothing to verify a payload against.
  if (state.bundle?.createUpdaterArtifacts !== true) {
    errors.push('Tauri updater artifacts must be enabled so a release can be installed in place')
  }
  const endpoints = state.updater?.endpoints ?? []
  if (endpoints.length === 0) errors.push('the updater has no endpoint to check')
  for (const endpoint of endpoints) {
    if (!endpoint.startsWith('https://')) {
      errors.push(`updater endpoint must be HTTPS: ${endpoint}`)
    }
  }
  if (typeof state.updater?.pubkey !== 'string' || state.updater.pubkey.trim() === '') {
    errors.push('the updater has no public key, so it would accept an unsigned payload')
  }
  for (const icon of state.iconPaths) {
    try {
      await readFile(icon)
    } catch {
      errors.push(`bundle icon is missing: ${path.relative(root, icon)}`)
    }
  }
  return { state, errors }
}

function replaceOnce(text, pattern, replacement, label) {
  if (!pattern.test(text)) throw new Error(`Could not stamp ${label}`)
  return text.replace(pattern, replacement)
}

/**
 * Publish a set of already-serialised files with rollback on reported errors.
 * Every byte is staged beside its target first. Publication keeps each old
 * file as a same-directory backup until every rename succeeds; any failure
 * restores the replaced targets in reverse order.
 */
export async function publishFilesAtomically(files, options = {}) {
  const token = `${process.pid}-${randomUUID()}`
  const staged = []
  try {
    for (const file of files) {
      const temporary = `${file.target}.release-stamp-${token}.tmp`
      const backup = `${file.target}.release-stamp-${token}.bak`
      const metadata = await stat(file.target)
      const prepared = { ...file, temporary, backup, backedUp: false, published: false }
      staged.push(prepared)
      const mode = metadata.mode & 0o777
      await writeFile(temporary, file.data, { flag: 'wx', mode })
      await chmod(temporary, mode)
    }
  } catch (error) {
    await Promise.allSettled(staged.map((file) => rm(file.temporary, { force: true })))
    throw error
  }

  try {
    for (const [index, file] of staged.entries()) {
      await options.beforeReplace?.({ target: file.target, index })
      await rename(file.target, file.backup)
      file.backedUp = true
      await rename(file.temporary, file.target)
      file.published = true
    }
  } catch (error) {
    const rollbackErrors = []
    for (const file of [...staged].reverse()) {
      try {
        if (file.published) await rm(file.target, { force: true })
        if (file.backedUp) await rename(file.backup, file.target)
        await rm(file.temporary, { force: true })
      } catch (rollback) {
        rollbackErrors.push(`${file.target}: ${rollback.message}`)
      }
    }
    if (rollbackErrors.length) {
      throw new Error(`${error.message}; rollback also failed: ${rollbackErrors.join('; ')}`)
    }
    throw error
  }

  const cleanup = await Promise.allSettled(staged.map((file) => rm(file.backup, { force: true })))
  const failures = cleanup.filter((result) => result.status === 'rejected')
  if (failures.length) {
    throw new Error(
      'Version stamp completed, but one or more recovery backups could not be removed',
    )
  }
}

export async function stampAppVersion(root, version, options = {}) {
  if (!isSemVer(version)) throw new Error(`Not a SemVer app version: ${version}`)

  const releaseFile = path.join(root, 'release/versions.json')
  const packageFile = path.join(root, 'package.json')
  const packageLockFile = path.join(root, 'package-lock.json')
  const tauriFile = path.join(root, 'src-tauri/tauri.conf.json')
  const cargoFile = path.join(root, 'src-tauri/Cargo.toml')
  const cargoLockFile = path.join(root, 'src-tauri/Cargo.lock')

  const release = await readJson(releaseFile)
  const packageJson = await readJson(packageFile)
  const packageLock = await readJson(packageLockFile)
  let tauri = await readFile(tauriFile, 'utf8')
  let cargo = await readFile(cargoFile, 'utf8')
  let cargoLock = await readFile(cargoLockFile, 'utf8')

  release.appVersion = version
  packageJson.version = version
  packageLock.version = version
  if (!packageLock.packages?.['']) throw new Error('package-lock.json has no root package')
  packageLock.packages[''].version = version
  tauri = replaceOnce(
    tauri,
    /("version"\s*:\s*")[^"]+(")/,
    `$1${version}$2`,
    'Tauri application version',
  )
  cargo = replaceOnce(
    cargo,
    /(\[workspace\.package\][\s\S]*?\nversion = ")[^"]+(")/,
    `$1${version}$2`,
    'Cargo workspace version',
  )
  let lockPackagesStamped = 0
  cargoLock = cargoLock.replace(
    /(\[\[package\]\]\r?\nname = "wobu(?:-[a-z0-9-]+)?"\r?\nversion = ")[^"]+(")/g,
    (_whole, before, after) => {
      lockPackagesStamped += 1
      return `${before}${version}${after}`
    },
  )
  if (lockPackagesStamped === 0) {
    throw new Error('Cargo.lock has no Wobu workspace package to stamp')
  }

  await publishFilesAtomically(
    [
      { target: releaseFile, data: json(release) },
      { target: packageFile, data: json(packageJson) },
      { target: packageLockFile, data: json(packageLock) },
      { target: tauriFile, data: tauri },
      { target: cargoFile, data: cargo },
      { target: cargoLockFile, data: cargoLock },
    ],
    options,
  )
}

async function main(argv) {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
  if (argv[0] === '--set') {
    if (!argv[1] || argv.length !== 2) throw new Error('Usage: npm run release:set -- <semver>')
    await stampAppVersion(root, argv[1])
  } else if (argv.length && !(argv.length === 1 && argv[0] === '--check')) {
    throw new Error('Usage: npm run release:check OR npm run release:set -- <semver>')
  }

  const { state, errors } = await checkRelease(root)
  if (errors.length) throw new Error(`Release check failed:\n- ${errors.join('\n- ')}`)
  process.stdout.write(
    `release ${state.declared.appVersion}; project schema ${state.projectSchemaVersion}; index schema ${state.indexSchemaVersion}; updater signed\n`,
  )
}

const invoked = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
if (invoked) {
  main(process.argv.slice(2)).catch((error) => {
    process.stderr.write(`${error.message}\n`)
    process.exitCode = 1
  })
}
