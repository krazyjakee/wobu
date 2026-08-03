/**
 * The MCP command surface.
 *
 * Kept out of `api.ts` deliberately. Everything in that file is something Wobu
 * does as a matter of course; everything in this one only happens because
 * somebody went to Settings and turned it on, and the separation is worth the
 * extra import for exactly that reason — nothing here is reachable from a
 * default install.
 *
 * Argument keys are camelCase and Tauri matches them to the snake_case
 * parameters in `src-tauri/src/mcp.rs`.
 */
import { invoke } from '@tauri-apps/api/core'

/** One tool the server can advertise. The disclosure is rendered from these. */
export interface McpTool {
  name: string
  title: string
  description: string
  /** Whether it changes the project. The second opt-in gates exactly these. */
  write: boolean
}

export interface McpServerView {
  enabled: boolean
  /** Whether a socket is actually open. Differs from `enabled` if the port was taken. */
  running: boolean
  port: number
  endpoint: string | null
  allowWrites: boolean
  /** Six characters and an ellipsis. The whole token needs `mcpServerToken()`. */
  tokenPreview: string | null
  error: string | null
}

export interface McpClientServer {
  id: string
  name: string
  command: string
  args: string[]
  enabled: boolean
  /** Environment overrides exist for this server. Their values never cross the bridge. */
  hasEnv: boolean
}

export interface McpClientView {
  enabled: boolean
  servers: McpClientServer[]
}

export interface McpSettings {
  server: McpServerView
  client: McpClientView
  tools: McpTool[]
}

/** One line of the activity log. Also the payload of the `mcp:activity` event. */
export interface McpActivity {
  at: string
  tool: string
  write: boolean
  ok: boolean
  detail: string | null
}

export interface McpRemoteTool {
  name: string
  title?: string
  description?: string
  inputSchema: unknown
}

export interface McpRemoteServer {
  id: string
  name: string
  version?: string
  protocolVersion: string
  tools: McpRemoteTool[]
}

/** Emitted for every tool call an agent makes, refused or not. */
export const MCP_ACTIVITY = 'mcp:activity'

export function mcpSettings(): Promise<McpSettings> {
  return invoke('mcp_settings')
}

/**
 * Change one thing at a time. Every field is optional because sending `port`
 * along with a toggle would let a half-typed number ride in on a click.
 */
export function mcpServerSet(patch: {
  enabled?: boolean
  port?: number
  allowWrites?: boolean
}): Promise<McpSettings> {
  return invoke('mcp_server_set', patch)
}

export function mcpServerToken(): Promise<string> {
  return invoke('mcp_server_token')
}

export function mcpServerTokenRotate(): Promise<McpSettings> {
  return invoke('mcp_server_token_rotate')
}

export function mcpActivity(): Promise<McpActivity[]> {
  return invoke('mcp_activity')
}

export function mcpClientSet(enabled: boolean): Promise<McpSettings> {
  return invoke('mcp_client_set', { enabled })
}

export function mcpClientServerUpsert(server: {
  id?: string
  name: string
  command: string
  args: string[]
  enabled: boolean
}): Promise<McpSettings> {
  return invoke('mcp_client_server_upsert', { server })
}

export function mcpClientServerRemove(id: string): Promise<McpSettings> {
  return invoke('mcp_client_server_remove', { id })
}

export function mcpClientServerProbe(id: string): Promise<McpRemoteServer> {
  return invoke('mcp_client_server_probe', { id })
}

/**
 * The JSON most MCP clients want, ready to paste.
 *
 * Built here rather than in Rust so the token only crosses the bridge when the
 * user presses the button that needs it.
 */
export function mcpClientConfigSnippet(endpoint: string, token: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        wobu: { type: 'http', url: endpoint, headers: { Authorization: `Bearer ${token}` } },
      },
    },
    null,
    2,
  )
}
