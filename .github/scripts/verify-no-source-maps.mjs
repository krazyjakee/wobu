import { readdir } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'

export async function findSourceMaps(root) {
  const matches = []

  async function visit(directory) {
    let entries
    try {
      entries = await readdir(directory, { withFileTypes: true })
    } catch (error) {
      if (error?.code === 'ENOENT') {
        throw new Error(`Release asset directory does not exist: ${root}`)
      }
      throw error
    }

    for (const entry of entries) {
      const file = path.join(directory, entry.name)
      if (entry.isDirectory()) {
        await visit(file)
      } else if (entry.name.toLowerCase().endsWith('.map')) {
        matches.push(path.relative(root, file).split(path.sep).join('/'))
      }
    }
  }

  await visit(root)
  return matches.sort()
}

export async function verifyNoSourceMaps(root) {
  const matches = await findSourceMaps(root)
  if (matches.length > 0) {
    throw new Error(
      `Release assets contain source maps:\n${matches.map((file) => `- ${file}`).join('\n')}`,
    )
  }
}

const invokedFile = process.argv[1] ? pathToFileURL(path.resolve(process.argv[1])).href : undefined
if (import.meta.url === invokedFile) {
  const root = path.resolve(process.argv[2] ?? 'dist')
  try {
    await verifyNoSourceMaps(root)
    console.log(`Verified release assets contain no source maps: ${root}`)
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = 1
  }
}
