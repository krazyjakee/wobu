import { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import {
  MCP_ACTIVITY,
  mcpActivity,
  mcpClientConfigSnippet,
  mcpClientServerProbe,
  mcpClientServerRemove,
  mcpClientServerUpsert,
  mcpClientSet,
  mcpServerSet,
  mcpServerToken,
  mcpServerTokenRotate,
  mcpSettings,
  type McpActivity,
  type McpSettings,
} from '../lib/mcp'
import { report, toast } from '../store/ui'
import { ConfirmSheet } from './ConfirmSheet'
import { Icon } from './Icon'

/**
 * Agent access, in the pane where somebody decides whether to have any.
 *
 * Two features live here and they are opposites: a *server* that lets an agent
 * on this computer read the open world, and a *client* that lets Wobu use MCP
 * servers the user already runs. Both start off, and this component is the only
 * thing in the app that can turn either on.
 *
 * ## Why this pane is written the way it is
 *
 * Wobu's privacy claim is a list of the places bytes can go. Everything else in
 * Settings picks *how* Wobu does something it was already going to do; this
 * pane adds a listening socket and the ability to launch a program. So the
 * copy is unusually blunt, the switches are three separate decisions rather
 * than one, and the disclosure is *generated* — the list of what an agent can
 * read comes from `settings.tools`, which is the same catalogue the protocol
 * advertises, so a tool added in Rust cannot quietly miss the sentence the user
 * reads.
 *
 * The three decisions, in order and independent:
 *
 * 1. Let an agent read this world (opens a loopback port, token-gated).
 * 2. Let it change this world (a second tick, confirmed, and until it is on the
 *    write tools are not even advertised — see `wobu-mcp`'s dispatcher).
 * 3. Let Wobu use servers you run (launches programs the user names).
 *
 * The activity list underneath is not decoration. A disclosure nobody can check
 * is a promise, and this is the part that makes it an observation: every call,
 * refused or not, appears here and in the diagnostics log.
 */
export default function McpSection() {
  const [settings, setSettings] = useState<McpSettings | null>(null)
  const [activity, setActivity] = useState<McpActivity[]>([])
  const [busy, setBusy] = useState(false)
  const [token, setToken] = useState<string | null>(null)
  const [confirmWrites, setConfirmWrites] = useState(false)
  const alive = useRef(true)

  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
    }
  }, [])

  const load = useCallback(async () => {
    try {
      const [next, log] = await Promise.all([mcpSettings(), mcpActivity()])
      if (!alive.current) return
      setSettings(next)
      setActivity(log)
    } catch (reason) {
      report(reason, 'Could not read the agent-access settings')
    }
  }, [])

  // The first read, written as a promise callback rather than `void load()` so
  // that nothing sets state synchronously inside an effect body — the same
  // shape the Storage pane uses.
  useEffect(() => {
    let disposed = false
    void Promise.all([mcpSettings(), mcpActivity()]).then(
      ([next, log]) => {
        if (disposed) return
        setSettings(next)
        setActivity(log)
      },
      (error: unknown) => report(error, 'Could not read the agent-access settings'),
    )
    return () => {
      disposed = true
    }
  }, [])

  // Live rather than polled: a call that arrives while the pane is open is
  // exactly the moment somebody wants to see it.
  useEffect(() => {
    const stop = listen<McpActivity>(MCP_ACTIVITY, (event) => {
      setActivity((current) => [event.payload, ...current].slice(0, 50))
    })
    return () => {
      void stop.then((off) => {
        off()
      })
    }
  }, [])

  /** Every mutation goes through here so no two are ever in flight at once. */
  async function run(change: () => Promise<McpSettings>, context: string) {
    setBusy(true)
    try {
      const next = await change()
      if (alive.current) setSettings(next)
    } catch (reason) {
      report(reason, context)
      // The switch may have moved even though the listener did not come up, so
      // the pane re-reads rather than assuming its own optimistic state.
      await load()
    } finally {
      if (alive.current) setBusy(false)
    }
  }

  function setServerEnabled(enabled: boolean) {
    if (!enabled) setToken(null)
    void run(() => mcpServerSet({ enabled }), 'Could not change the MCP server')
  }

  function setAllowWrites(allowWrites: boolean) {
    void run(() => mcpServerSet({ allowWrites }), 'Could not change MCP write access')
  }

  async function reveal() {
    try {
      setToken(await mcpServerToken())
    } catch (reason) {
      report(reason, 'Could not read the MCP token')
    }
  }

  async function copyConfig() {
    if (!settings?.server.endpoint) return
    try {
      const secret = token ?? (await mcpServerToken())
      await navigator.clipboard.writeText(mcpClientConfigSnippet(settings.server.endpoint, secret))
      toast('The MCP connection details were copied.')
    } catch (reason) {
      report(reason, 'Could not copy the MCP connection details')
    }
  }

  if (!settings) {
    return (
      <section className="set-sec" aria-labelledby="mcp-title">
        <h3 id="mcp-title">Agent access (MCP)</h3>
        <p className="set-note">Reading the agent-access settings…</p>
      </section>
    )
  }

  const { server, client, tools } = settings
  const readTools = tools.filter((tool) => !tool.write)
  const writeTools = tools.filter((tool) => tool.write)

  return (
    <section className="set-sec" aria-labelledby="mcp-title">
      <h3 id="mcp-title">Agent access (MCP)</h3>
      <p className="set-note">
        Off by default, and off in two directions. Until you tick something here Wobu listens on no
        port and runs no other program. Nothing on this pane sends anything to us or to anyone else
        — both switches are about this computer.
      </p>

      {/* ── server ─────────────────────────────────────────────────────── */}

      <div className="set-row set-row-col">
        <label className="set-value">
          <input
            type="checkbox"
            checked={server.enabled}
            disabled={busy}
            onChange={(event) => setServerEnabled(event.target.checked)}
          />{' '}
          Let an agent on this computer read the open world
        </label>
      </div>
      <p className="set-note">
        Opens a port on <code>127.0.0.1</code> only — never your network — and refuses every request
        that does not carry the access token below. An agent that connects can read:{' '}
        {readTools.map((tool) => tool.title.toLowerCase()).join(', ')}. It cannot change anything
        and cannot start a generation, so nothing it does can spend money.
      </p>

      {server.error && (
        <p className="wiki-export-error" role="alert">
          {server.error}
        </p>
      )}

      {server.enabled && (
        <>
          <div className="set-row">
            <span className="set-label">Address</span>
            <code className="set-path">
              {server.endpoint ?? `not listening (port ${server.port})`}
            </code>
          </div>
          <div className="set-row">
            <span className="set-label">Token</span>
            <code className="set-path">{token ?? server.tokenPreview ?? '—'}</code>
          </div>
          {/* Keyed on the current port so a value the backend refused or
              normalised resets the field, rather than leaving the box arguing
              with the address above it. */}
          <PortField
            key={server.port}
            port={server.port}
            busy={busy}
            onApply={(next) =>
              void run(() => mcpServerSet({ port: next }), 'Could not change the MCP port')
            }
          />
          <div className="set-acts">
            <button className="btn-mini" onClick={() => void reveal()} disabled={token !== null}>
              <Icon name="lock" size="sm" />
              Show token
            </button>
            <button className="btn-mini" onClick={() => void copyConfig()}>
              <Icon name="copy" size="sm" />
              Copy connection details
            </button>
            <button
              className="btn-mini"
              disabled={busy}
              onClick={() => {
                setToken(null)
                void run(mcpServerTokenRotate, 'Could not make a new MCP token')
              }}
            >
              <Icon name="refresh" size="sm" />
              New token
            </button>
          </div>

          <div className="set-row set-row-col">
            <label className="set-value">
              <input
                type="checkbox"
                checked={server.allowWrites}
                disabled={busy}
                onChange={(event) => {
                  if (event.target.checked) setConfirmWrites(true)
                  else setAllowWrites(false)
                }}
              />{' '}
              Let a connected agent change this world
            </label>
          </div>
          <p className="set-note">
            {server.allowWrites
              ? `On. The agent can also ${writeTools
                  .map((tool) => tool.title.toLowerCase())
                  .join(', ')} — every one of those writes a file in your project folder.`
              : 'Off. The write tools are not offered to the agent at all, so it will not try.'}
          </p>
        </>
      )}

      {/* ── client ─────────────────────────────────────────────────────── */}

      <div className="set-row set-row-col">
        <label className="set-value">
          <input
            type="checkbox"
            checked={client.enabled}
            disabled={busy}
            onChange={(event) =>
              void run(() => mcpClientSet(event.target.checked), 'Could not change MCP clients')
            }
          />{' '}
          Let Wobu use MCP servers you run
        </label>
      </div>
      <p className="set-note">
        Each server below is a program on this computer that Wobu will start as you, with the
        arguments you give it. Nothing is launched until both this switch and the server&apos;s own
        are on.
      </p>

      {client.enabled && (
        <ClientServers
          settings={settings}
          busy={busy}
          onChanged={setSettings}
          onBusy={setBusy}
          onReload={load}
        />
      )}

      {/* ── activity ───────────────────────────────────────────────────── */}

      {(server.enabled || client.enabled) && (
        <>
          <div className="set-row">
            <span className="set-label">Activity</span>
            <span className="set-value">
              {activity.length === 0
                ? 'Nothing has called Wobu yet.'
                : `The last ${activity.length} tool ${activity.length === 1 ? 'call' : 'calls'}.`}
            </span>
          </div>
          {activity.length > 0 && (
            <pre className="set-log" aria-label="Recent agent activity">
              {activity
                .map(
                  (entry) =>
                    `${entry.at}  ${entry.ok ? 'ok      ' : 'refused '} ${entry.tool}${
                      entry.detail ? `  — ${entry.detail}` : ''
                    }`,
                )
                .join('\n')}
            </pre>
          )}
        </>
      )}

      {confirmWrites && (
        <ConfirmSheet
          title="Let an agent change this world?"
          body={
            `A connected agent will be able to ${writeTools
              .map((tool) => tool.title.toLowerCase())
              .join(', ')}. Those write Markdown files in your project folder, and on a shared ` +
            'project your collaborators will see them. Nothing is deleted, every call is listed ' +
            'below, and you can turn this off again at any time.'
          }
          confirmLabel="Allow changes"
          danger
          busy={busy}
          onCancel={() => setConfirmWrites(false)}
          onConfirm={() => {
            setConfirmWrites(false)
            setAllowWrites(true)
          }}
        />
      )}
    </section>
  )
}

/**
 * The port, which is the only part of the address there is to choose.
 *
 * There is no host field, and that is the point rather than an omission: the
 * listener binds `127.0.0.1` in one place in `wobu-mcp` and no setting reaches
 * it. Applied on blur or Enter rather than per keystroke, because every apply
 * closes the socket and opens a new one.
 */
function PortField({
  port,
  busy,
  onApply,
}: {
  port: number
  busy: boolean
  onApply: (port: number) => void
}) {
  const [draft, setDraft] = useState(String(port))
  const parsed = Number(draft)
  const valid = Number.isInteger(parsed) && parsed >= 1024 && parsed <= 65535

  function apply() {
    if (!valid || parsed === port) return
    onApply(parsed)
  }

  return (
    <div className="set-row">
      <label className="set-label" htmlFor="mcp-port">
        Port
      </label>
      <input
        id="mcp-port"
        type="number"
        min={1024}
        max={65535}
        value={draft}
        disabled={busy}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={apply}
        onKeyDown={(event) => {
          if (event.key === 'Enter') apply()
        }}
      />
      <span className="set-value">
        {valid ? 'on 127.0.0.1 only, never your network' : 'Pick a port between 1024 and 65535.'}
      </span>
    </div>
  )
}

/**
 * The configured servers, and the form for adding one.
 *
 * Split out so the switch above stays readable: this half is a list editor and
 * has nothing to say about the trust decision that gates it.
 */
function ClientServers({
  settings,
  busy,
  onChanged,
  onBusy,
  onReload,
}: {
  settings: McpSettings
  busy: boolean
  onChanged: (settings: McpSettings) => void
  onBusy: (busy: boolean) => void
  onReload: () => Promise<void>
}) {
  const [name, setName] = useState('')
  const [command, setCommand] = useState('')
  const [args, setArgs] = useState('')
  const [probed, setProbed] = useState<Record<string, string>>({})

  async function run(change: () => Promise<McpSettings>, context: string) {
    onBusy(true)
    try {
      onChanged(await change())
    } catch (reason) {
      report(reason, context)
      await onReload()
    } finally {
      onBusy(false)
    }
  }

  function add() {
    if (!command.trim()) return
    void run(
      () =>
        mcpClientServerUpsert({
          name: name.trim() || command.trim(),
          command: command.trim(),
          // Split on whitespace, which is what a person types. Anything needing
          // a quoted argument can be edited in `mcp.json`.
          args: args.trim() ? args.trim().split(/\s+/) : [],
          enabled: false,
        }),
      'Could not add that MCP server',
    ).then(() => {
      setName('')
      setCommand('')
      setArgs('')
    }, undefined)
  }

  async function probe(id: string) {
    onBusy(true)
    try {
      const found = await mcpClientServerProbe(id)
      setProbed((current) => ({
        ...current,
        [id]: `${found.name} answered with ${found.tools.length} tool${
          found.tools.length === 1 ? '' : 's'
        }: ${found.tools.map((tool) => tool.name).join(', ') || 'none'}`,
      }))
    } catch (reason) {
      report(reason, 'That MCP server did not answer')
    } finally {
      onBusy(false)
    }
  }

  return (
    <>
      {settings.client.servers.map((server) => (
        <div className="set-row set-row-col" key={server.id}>
          <label className="set-value">
            <input
              type="checkbox"
              checked={server.enabled}
              disabled={busy}
              onChange={(event) =>
                void run(
                  () =>
                    mcpClientServerUpsert({
                      id: server.id,
                      name: server.name,
                      command: server.command,
                      args: server.args,
                      enabled: event.target.checked,
                    }),
                  'Could not change that MCP server',
                )
              }
            />{' '}
            {server.name}
          </label>
          <code className="set-path">
            {[server.command, ...server.args].join(' ')}
            {server.hasEnv ? '  (with environment overrides from mcp.json)' : ''}
          </code>
          {probed[server.id] && <span className="set-value">{probed[server.id]}</span>}
          <div className="set-acts">
            <button
              className="btn-mini"
              disabled={busy || !server.enabled}
              onClick={() => void probe(server.id)}
            >
              <Icon name="refresh" size="sm" />
              Check it works
            </button>
            <button
              className="btn-mini"
              disabled={busy}
              onClick={() =>
                void run(() => mcpClientServerRemove(server.id), 'Could not remove that MCP server')
              }
            >
              <Icon name="trash" size="sm" />
              Remove
            </button>
          </div>
        </div>
      ))}

      <div className="set-row set-row-col">
        <label className="set-label" htmlFor="mcp-server-name">
          Add a server
        </label>
        <input
          id="mcp-server-name"
          value={name}
          placeholder="Name"
          onChange={(event) => setName(event.target.value)}
        />
        <input
          id="mcp-server-command"
          aria-label="Command"
          value={command}
          placeholder="Command, e.g. /usr/local/bin/mcp-notes"
          onChange={(event) => setCommand(event.target.value)}
        />
        <input
          id="mcp-server-args"
          aria-label="Arguments"
          value={args}
          placeholder="Arguments, separated by spaces"
          onChange={(event) => setArgs(event.target.value)}
        />
      </div>
      <div className="set-acts">
        <button className="btn-mini" disabled={busy || !command.trim()} onClick={add}>
          <Icon name="plus" size="sm" />
          Add server
        </button>
      </div>
      <p className="set-note">
        A server is added switched off. Turn it on when you are ready for Wobu to run it.
      </p>
    </>
  )
}
