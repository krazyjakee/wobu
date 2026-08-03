# 05 — Technical Architecture

## Stack

- **Shell**: Tauri 2 (Rust core, system webview).
- **Frontend**: React 19 + TypeScript + Vite. Chosen over Svelte for ecosystem depth in the
  things this app leans on hardest — virtualised image grids, drag-and-drop trees, and
  three.js integration. Styling is plain CSS with the design tokens from
  [03](03-ui-layout.md), keeping the implemented layout aligned with the design contract.
- **State**: Zustand for UI state; TanStack Query over Tauri commands for world data.
- **Rust**: workspace of small crates, so the domain logic is testable without a webview.

```
src-tauri/
├── src/main.rs              tauri::Builder, command + event registration
└── crates/
    ├── wobu-core/           Node/Link/Asset types, kind registry, validation
    ├── wobu-store/          Markdown+frontmatter IO, SQLite index, file watcher
    ├── wobu-influence/      stack resolution, fragment scoring, prompt compilation
    ├── wobu-llm/            Anthropic client, tool-use schemas, streaming Enhance
    ├── wobu-imagine/        image/3D backend adapters behind one trait
    ├── wobu-jobs/           queue, cancellation, progress events, retries
    └── wobu-mcp/            Model Context Protocol, both directions, off until enabled
```

`wobu-influence` is pure — stack in, compiled prompt out, no IO. It gets snapshot tests over
fixture worlds, because prompt-compilation regressions are otherwise invisible.

## Backend adapters

One trait, several implementations, selected per project:

```rust
#[async_trait]
pub trait ImageBackend {
    fn capabilities(&self) -> Capabilities;      // controlnet? ip-adapter? loras? max res
    async fn submit(&self, job: ImageJob) -> Result<JobHandle>;
    async fn progress(&self, h: &JobHandle) -> BoxStream<Progress>;
    async fn cancel(&self, h: &JobHandle) -> Result<()>;
}
```

- **ComfyUI (local)** — the primary target, given the machine has a real GPU. Wobu ships
  workflow templates per output preset and patches node inputs by ID; talks over
  `/prompt` + the websocket for progress and previews.
- **Replicate / fal** — hosted fallback for machines without a GPU, same trait.
- **Automatic1111** — optional, community request.

Capabilities surface provider counting budgets separately from adapter mechanisms. If a backend
has no structure mechanism, structure references are visibly downgraded to moodboard-only; if a
one-input ControlNet graph is full, the extra reference is attributed to that mechanism limit.

3D uses the same trait shape (`MeshBackend`): turnaround sheet → image-to-3D (TRELLIS /
Hunyuan3D / InstantMesh via ComfyUI) → GLB, previewed in-app with three.js.

## Commands and events

Commands are coarse and domain-shaped, not CRUD-shaped:

```
project_open / project_create / project_recent
node_list / node_get / node_upsert / node_delete / node_move
asset_import / asset_thumb / asset_link
influence_resolve(node_id)        -> layer cards for the Inspector
prompt_compile(node_id, shot)     -> compiled prompt + fragment attribution
enhance_start(node_id)            -> job id; streams via event
generate_start(node_id, shot)     -> job id
job_cancel(job_id)
```

Everything long-running returns a job id immediately and streams over Tauri events:
`job:progress`, `job:preview` (latent previews from ComfyUI), `job:done`, `job:error`,
`enhance:delta`. The frontend never blocks on a command.

`prompt_compile` is called on every Inspector interaction, so it must stay sub-millisecond —
another reason `wobu-influence` does no IO.

## Agent access (MCP)

`wobu-mcp` is the only crate that can open a listening socket or start another program, and it
does neither unless a setting says a person asked for it. The server binds `127.0.0.1` — the
port is configurable and the address is not — and refuses any request without its bearer token
or with an `Origin` header. Read tools are available whenever it is on; the three write tools
are a second, independent opt-in and are not even advertised until it is granted. The client
half launches stdio MCP servers the user named, directly rather than through a shell, and only
when both the master switch and that server's own switch are on.

`src-tauri/src/mcp.rs` holds the settings (`mcp.json` in app data, `0600`, never in a project),
the listener handle — dropping it is what "off" means — and the implementation of the crate's
`World` trait against the open project. Every tool call is emitted as `mcp:activity` and
written to the diagnostics log. Full detail, including what is deliberately not implemented,
in [16 — Agent Access (MCP)](16-mcp.md).

## File watching and the index

`wobu-store` reconciles external edits (Obsidian, git pull, a collaborator on the same share)
into the SQLite index and pushes a `world:changed` event. It picks its change-detection
strategy from the project path:

- **Local filesystem** → `notify` watcher, debounced ~400 ms.
- **Network mount** → 5-second directory-listing poll, because `inotify`/`FSEvents` do not
  observe writes made by other hosts over NFS or SMB.

Writes stage to `.wobu/tmp` on the same filesystem and `rename()` into place, guarded by an
mtime+hash check that converts a clobber into a surfaced conflict. Details in
[07 — Projects on file shares](07-file-shares.md).

The index holds a mirror of node frontmatter, an FTS5 table over notes and descriptions, and
the link edges. It lives in **local app data keyed by project ULID**, never in the project
folder — SQLite's locking is unsafe over SMB/NFS and WAL mode doesn't work there at all.
Schema or hash mismatch triggers a rebuild from Markdown; deleting the index is always safe.

## Static world wiki export

`project_export_wiki` renders the open project to a new folder outside the project. The result is
a read-only projection, not another canonical store: it contains grouped index and node pages,
reference and concept galleries, an SVG influence graph, copied image originals/thumbnails, and
one stylesheet. Every link is relative, so the folder can be browsed from disk or uploaded to an
ordinary static host without a server, JavaScript, or a database.

The exporter reconciles and clones nodes/assets while holding the project lock, then releases it
before strictly reading generation receipts, copying media, and rendering. Malformed receipts fail
closed before the destination is claimed; missing image blobs instead produce visible placeholders
and a warning count because the rest of the world remains useful. User-authored text is escaped in
text and attribute contexts, with only a small escaped Markdown block subset rendered as HTML.

Export paths are intentionally one-shot. The destination must not exist, its existing parent is
canonicalised before checking that it lies outside the project, and no previous export is ever
overwritten. A `.wobu-export-incomplete` marker is created first, `index.html` is written last, and
the marker is removed only after success. A failed export is left visibly incomplete and
recoverable; the exporter never cleans up or deletes user data.

## Concerns worth naming early

- **Secrets**: BYOK API keys go in the OS keychain via `keyring`, never in `project.json`.
  This is non-negotiable now that project folders are meant to be shared — see
  [08 — Providers & BYOK](08-providers.md).
- **Thumbnails**: generate WebP thumbs on import off the UI thread; grids bind to thumbs only.
- **Cancellation**: every job must be genuinely cancellable, including in-flight ComfyUI
  prompts, or the queue becomes a hostage situation.
- **Undo**: node edits go through a command log so `⌘Z` works across the whole workspace, not
  just inside a text field.
