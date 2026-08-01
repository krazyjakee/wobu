import { readFile } from 'node:fs/promises'

const tag = process.argv[2] ?? process.env.GITHUB_REF_NAME
const semver =
  /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/

if (!tag || !semver.test(tag)) {
  console.error(
    `release tag must be vMAJOR.MINOR.PATCH (optionally with a SemVer suffix), got ${tag ?? 'nothing'}`,
  )
  process.exit(1)
}

const expected = tag.slice(1)
const read = (path) => readFile(new URL(`../../${path}`, import.meta.url), 'utf8')
const parseJson = async (path) => JSON.parse(await read(path))
const [packageJson, packageLock, tauriConfig, cargoToml, cargoLock, core, index] =
  await Promise.all([
    parseJson('package.json'),
    parseJson('package-lock.json'),
    parseJson('src-tauri/tauri.conf.json'),
    read('src-tauri/Cargo.toml'),
    read('src-tauri/Cargo.lock'),
    read('src-tauri/crates/wobu-core/src/lib.rs'),
    read('src-tauri/crates/wobu-store/src/index.rs'),
  ])

const cargoVersion = cargoToml.match(/\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1]
const projectSchema = core.match(/pub const SCHEMA_VERSION: u32 = (\d+);/)?.[1]
const indexSchema = index.match(/pub const INDEX_VERSION: u32 = (\d+);/)?.[1]

const versions = [
  ['package.json', packageJson.version],
  ['package-lock.json', packageLock.version],
  ['package-lock.json root package', packageLock.packages?.['']?.version],
  ['src-tauri/tauri.conf.json', tauriConfig.version],
  ['src-tauri/Cargo.toml workspace', cargoVersion],
]

for (const block of cargoLock.matchAll(/\[\[package\]\]\n([\s\S]*?)(?=\n\[\[package\]\]|$)/g)) {
  const name = block[1].match(/^name = "([^"]+)"/m)?.[1]
  const version = block[1].match(/^version = "([^"]+)"/m)?.[1]
  const isWorkspacePackage = !/^source = /m.test(block[1])
  if (isWorkspacePackage && /^wobu(?:-|$)/.test(name ?? '')) {
    versions.push([`src-tauri/Cargo.lock package ${name}`, version])
  }
}

const mismatches = versions.filter(([, version]) => version !== expected)
if (mismatches.length > 0) {
  console.error(`tag ${tag} does not match every application version:`)
  for (const [file, version] of versions) {
    console.error(`  ${file}: ${version ?? 'missing'}`)
  }
  process.exit(1)
}

if (!projectSchema || !indexSchema) {
  console.error('could not read the project and index schema versions')
  process.exit(1)
}

console.log(
  `release versions are coherent: app ${expected}, project schema ${projectSchema}, index schema ${indexSchema}`,
)
