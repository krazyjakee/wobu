/**
 * Serves `website/dist` for a local look at the built site. Dependency-free on
 * purpose: the site has no build server, and `npx serve` would pull a tree down
 * just to read files off disk.
 *
 *   node preview.mjs [--port 4173] [--dir dist]
 */
import { createReadStream } from 'node:fs'
import { stat } from 'node:fs/promises'
import { createServer } from 'node:http'
import { extname, join, normalize, resolve } from 'node:path'
import { dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))

function flag(name, fallback) {
  const index = process.argv.indexOf(`--${name}`)
  return index === -1 ? fallback : process.argv[index + 1]
}

const root = resolve(here, flag('dir', 'dist'))
const port = Number(flag('port', '4173'))

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.webp': 'image/webp',
  '.xml': 'application/xml; charset=utf-8',
  '.txt': 'text/plain; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
}

async function resolveFile(pathname) {
  const relative = normalize(decodeURIComponent(pathname)).replace(/^(\.\.[/\\])+/, '')
  let candidate = join(root, relative)
  if (!candidate.startsWith(root)) return null

  try {
    const stats = await stat(candidate)
    if (stats.isDirectory()) candidate = join(candidate, 'index.html')
    else return candidate
  } catch {
    if (!extname(candidate)) candidate = `${candidate}.html`
    else return null
  }

  try {
    await stat(candidate)
    return candidate
  } catch {
    return null
  }
}

const server = createServer(async (request, response) => {
  const { pathname } = new URL(request.url, 'http://localhost')
  const file = await resolveFile(pathname)

  if (!file) {
    response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' })
    response.end(`404 — ${pathname}\n`)
    return
  }

  response.writeHead(200, { 'content-type': TYPES[extname(file)] ?? 'application/octet-stream' })
  createReadStream(file).pipe(response)
})

server.listen(port, () => {
  console.log(`Wobu site preview on http://localhost:${port}/  (serving ${root})`)
})
