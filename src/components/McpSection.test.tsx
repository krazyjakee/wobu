import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import McpSection from './McpSection'
import type { McpSettings } from '../lib/mcp'

/*
 * The one pane in Wobu that can open a socket and start a program, so the
 * assertions here are about what does *not* happen. Every test asserts on the
 * commands that reached the backend rather than on markup: the promise this
 * feature makes is "nothing is listening until you say so", and the only way to
 * check it from the renderer is that nothing was ever asked to listen.
 */

const h = vi.hoisted(() => ({ invoke: vi.fn(), writeText: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: h.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

/** The catalogue as the Rust side reports it, trimmed to what the copy uses. */
const TOOLS: McpSettings['tools'] = [
  { name: 'world_overview', title: 'World overview', description: 'Counts.', write: false },
  { name: 'get_node', title: 'Read a node', description: 'One entity.', write: false },
  { name: 'compile_prompt', title: 'Compile the prompt', description: 'Text.', write: false },
  { name: 'create_node', title: 'Create a node', description: 'Writes.', write: true },
  { name: 'update_node', title: 'Update a node', description: 'Writes.', write: true },
  { name: 'link_nodes', title: 'Link two nodes', description: 'Writes.', write: true },
]

let settings: McpSettings

function fresh(): McpSettings {
  return {
    server: {
      enabled: false,
      running: false,
      port: 9628,
      endpoint: null,
      allowWrites: false,
      tokenPreview: null,
      error: null,
    },
    client: { enabled: false, servers: [] },
    tools: TOOLS,
  }
}

/** Commands that changed something, in order. Reads are uninteresting here. */
function mutations(): string[] {
  return h.invoke.mock.calls
    .map(([cmd]) => String(cmd))
    .filter((cmd) => cmd !== 'mcp_settings' && cmd !== 'mcp_activity')
}

beforeEach(() => {
  h.invoke.mockReset()
  h.writeText.mockReset()
  h.writeText.mockResolvedValue(undefined)
  Object.defineProperty(navigator, 'clipboard', {
    value: { writeText: h.writeText },
    configurable: true,
  })
  settings = fresh()

  h.invoke.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case 'mcp_settings':
        return Promise.resolve(settings)
      case 'mcp_activity':
        return Promise.resolve([])
      case 'mcp_server_set': {
        if (typeof args?.enabled === 'boolean') {
          settings.server.enabled = args.enabled
          settings.server.running = args.enabled
          settings.server.endpoint = args.enabled ? 'http://127.0.0.1:9628/mcp' : null
          settings.server.tokenPreview = args.enabled ? 'a1b2c3…' : null
        }
        if (typeof args?.allowWrites === 'boolean') settings.server.allowWrites = args.allowWrites
        return Promise.resolve(settings)
      }
      case 'mcp_server_token':
        return Promise.resolve('a1b2c3d4e5f6')
      case 'mcp_client_set':
        settings.client.enabled = Boolean(args?.enabled)
        return Promise.resolve(settings)
      case 'mcp_client_server_upsert': {
        const server = args?.server as {
          id?: string
          name: string
          command: string
          args: string[]
          enabled: boolean
        }
        settings.client.servers = [
          { id: server.id ?? 'new-id', ...server, args: server.args, hasEnv: false },
        ]
        return Promise.resolve(settings)
      }
      default:
        return Promise.reject(new Error(`unexpected command ${cmd}`))
    }
  })
})

describe('agent access (MCP)', () => {
  it('shows both halves switched off and asks the backend to start nothing', async () => {
    render(<McpSection />)

    const read = await screen.findByLabelText(/read the open world/i)
    expect(read).not.toBeChecked()
    expect(screen.getByLabelText(/MCP servers you run/i)).not.toBeChecked()

    // Off means off: no address, no token, and — the load-bearing one — no
    // command that could have opened a port.
    expect(screen.queryByText('http://127.0.0.1:9628/mcp')).not.toBeInTheDocument()
    expect(mutations()).toEqual([])
  })

  it('names exactly what an agent could read, from the catalogue rather than from prose', async () => {
    render(<McpSection />)
    await screen.findByLabelText(/read the open world/i)

    // Generated from `settings.tools`, so a tool added in Rust cannot miss the
    // disclosure. Read tools are named; write tools are not, because at this
    // point they are not offered at all.
    const disclosure = screen.getByText(/An agent that connects can read/i)
    expect(disclosure).toHaveTextContent('world overview')
    expect(disclosure).toHaveTextContent('compile the prompt')
    expect(disclosure).not.toHaveTextContent('create a node')
    expect(disclosure).toHaveTextContent(/cannot change anything/i)
  })

  it('opens the loopback endpoint only when the switch is ticked, and shows it', async () => {
    render(<McpSection />)
    fireEvent.click(await screen.findByLabelText(/read the open world/i))

    await waitFor(() => expect(mutations()).toEqual(['mcp_server_set']))
    expect(h.invoke).toHaveBeenCalledWith('mcp_server_set', { enabled: true })
    expect(await screen.findByText('http://127.0.0.1:9628/mcp')).toBeInTheDocument()
    // A preview, not the credential.
    expect(screen.getByText('a1b2c3…')).toBeInTheDocument()
    expect(screen.queryByText('a1b2c3d4e5f6')).not.toBeInTheDocument()
  })

  it('keeps the whole token off the pane until it is asked for', async () => {
    render(<McpSection />)
    fireEvent.click(await screen.findByLabelText(/read the open world/i))
    await screen.findByText('a1b2c3…')

    fireEvent.click(screen.getByRole('button', { name: /show token/i }))
    expect(await screen.findByText('a1b2c3d4e5f6')).toBeInTheDocument()
  })

  it('copies connection details an MCP client can actually use', async () => {
    render(<McpSection />)
    fireEvent.click(await screen.findByLabelText(/read the open world/i))
    await screen.findByText('a1b2c3…')

    fireEvent.click(screen.getByRole('button', { name: /copy connection details/i }))
    await waitFor(() => expect(h.writeText).toHaveBeenCalled())
    const copied = JSON.parse(String(h.writeText.mock.calls[0]?.[0])) as {
      mcpServers: { wobu: { url: string; headers: Record<string, string> } }
    }
    expect(copied.mcpServers.wobu.url).toBe('http://127.0.0.1:9628/mcp')
    expect(copied.mcpServers.wobu.headers.Authorization).toBe('Bearer a1b2c3d4e5f6')
  })

  it('lets the port be chosen and refuses a privileged one without asking the backend', async () => {
    settings.server = {
      ...settings.server,
      enabled: true,
      running: true,
      endpoint: 'http://127.0.0.1:9628/mcp',
      tokenPreview: 'a1b2c3…',
    }
    render(<McpSection />)

    const port = await screen.findByLabelText('Port')
    expect(port).toHaveValue(9628)

    fireEvent.change(port, { target: { value: '80' } })
    fireEvent.blur(port)
    expect(mutations()).toEqual([])
    expect(screen.getByText(/between 1024 and 65535/i)).toBeInTheDocument()

    fireEvent.change(port, { target: { value: '9700' } })
    fireEvent.blur(port)
    await waitFor(() => expect(h.invoke).toHaveBeenCalledWith('mcp_server_set', { port: 9700 }))
  })

  it('will not grant writes without a second, confirmed decision', async () => {
    render(<McpSection />)
    fireEvent.click(await screen.findByLabelText(/read the open world/i))
    const writes = await screen.findByLabelText(/change this world/i)
    expect(writes).not.toBeChecked()

    fireEvent.click(writes)
    // A dialog, not a toggle: the sheet names the three tools it is about.
    const sheet = await screen.findByRole('alertdialog')
    expect(sheet).toHaveTextContent('create a node')
    expect(sheet).toHaveTextContent('link two nodes')

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
    expect(mutations()).toEqual(['mcp_server_set'])
    expect(h.invoke).not.toHaveBeenCalledWith('mcp_server_set', { allowWrites: true })
  })

  it('grants writes once the sheet is confirmed, and says what that now allows', async () => {
    render(<McpSection />)
    fireEvent.click(await screen.findByLabelText(/read the open world/i))
    fireEvent.click(await screen.findByLabelText(/change this world/i))
    fireEvent.click(await screen.findByRole('button', { name: 'Allow changes' }))

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('mcp_server_set', { allowWrites: true }),
    )
    expect(await screen.findByText(/writes a file in your project folder/i)).toBeInTheDocument()
  })

  it('turning writes off again needs no confirmation and takes effect immediately', async () => {
    settings.server = {
      ...settings.server,
      enabled: true,
      running: true,
      endpoint: 'http://127.0.0.1:9628/mcp',
      tokenPreview: 'a1b2c3…',
      allowWrites: true,
    }
    render(<McpSection />)

    fireEvent.click(await screen.findByLabelText(/change this world/i))
    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('mcp_server_set', { allowWrites: false }),
    )
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
  })

  it('adds a server the user names switched off, so nothing is launched by adding it', async () => {
    render(<McpSection />)
    fireEvent.click(await screen.findByLabelText(/MCP servers you run/i))

    const command = await screen.findByLabelText('Command')
    fireEvent.change(command, { target: { value: '/usr/local/bin/mcp-notes' } })
    fireEvent.change(screen.getByLabelText('Arguments'), { target: { value: '--root /notes' } })
    fireEvent.click(screen.getByRole('button', { name: /add server/i }))

    await waitFor(() =>
      expect(h.invoke).toHaveBeenCalledWith('mcp_client_server_upsert', {
        server: {
          name: '/usr/local/bin/mcp-notes',
          command: '/usr/local/bin/mcp-notes',
          args: ['--root', '/notes'],
          enabled: false,
        },
      }),
    )
    // Nothing was probed and nothing was called: adding is not running.
    expect(mutations()).not.toContain('mcp_client_server_probe')
  })

  it('reports a listener that could not come up rather than showing a switch that lies', async () => {
    settings.server = {
      ...settings.server,
      enabled: true,
      running: false,
      error: 'nothing could listen on 127.0.0.1:9628 — something else already is',
    }
    render(<McpSection />)

    expect(await screen.findByRole('alert')).toHaveTextContent('something else already is')
    expect(screen.getByText(/not listening \(port 9628\)/)).toBeInTheDocument()
  })
})
