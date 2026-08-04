# Wobu

Wobu is a local-first, AI-assisted desktop app for building coherent fictional worlds and producing
consistent concept art. Define lore, visual style, species, cultures, places, characters, and props
once; Wobu resolves that hierarchy into an attributed prompt whenever you generate an image.

Projects are ordinary, self-contained `.wobu` directories. Notes remain readable Markdown, assets
remain files, and the disposable search index stays outside the project. Wobu can work with a local
ComfyUI installation or connect directly to supported providers using your own credentials—there is
no Wobu-operated inference proxy.

> **Beta software:** release bundles are currently unsigned. Only download them from this
> repository's [GitHub Releases](https://github.com/krazyjakee/wobu/releases) page and review the
> [platform-specific installation guidance](docs/12-releasing.md#installing-an-unsigned-beta).

## What Wobu does

- Organizes a world as a hierarchy of reusable influences instead of isolated prompts.
- Turns rough notes into editable, structured descriptions with Anthropic or Gemini.
- Compiles transparent prompts whose fragments remain attributable, mutable, and weightable.
- Uses reference images as silhouette, structure, palette, material, mood, or pose context.
- Generates concept images through local ComfyUI or Gemini, with history, replay, variants, and
  pin-to-reference workflows.
- Supports shared-folder collaboration, conflict-safe writes, and ticket-based peer-to-peer sync.
- Provides boards, relationship graphs, Forge workflows, wiki export, and concept-mesh viewing.

Concept 3D creation is still partial; the current UI can view and export existing meshes. See the
[roadmap](docs/09-roadmap.md) for exact feature status.

## Getting started

### Install a beta release

Download the bundle for Linux, macOS, or Windows from
[Releases](https://github.com/krazyjakee/wobu/releases). Builds are unsigned and update manually, so
follow the [release guide](docs/12-releasing.md) before bypassing an operating-system warning.

Once Wobu opens, create a project from the Launcher. Start with its **Style Guide** and **World
Canon**, add broad world layers before individual characters, then configure providers in Settings.
The included [user guide](docs/guide/index.md) covers the complete workflow. It is also read into
the app itself — press `F1`, or use the Guide button on the mode rail — and published to
<https://krazyjakee.github.io/wobu/guide/>.

### Run from source

Prerequisites:

- Node.js 22 and npm;
- the Rust toolchain (version `1.97` is pinned and installed automatically by `rustup`);
- the [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

```sh
git clone https://github.com/krazyjakee/wobu.git
cd wobu
npm ci
cd src-tauri && rustup toolchain install --no-self-update && cd ..
npm run tauri dev
```

`npm run dev` starts only the Vite frontend. Use `npm run tauri dev` for normal development so the
webview can reach the Rust commands. To launch through Cargo instead, use
`./scripts/cargo-run.sh`; it starts Vite, waits for port 1420, and runs `cargo run` from
`src-tauri/`.

## Provider configuration

Credentials entered in the app are stored per installation in the operating-system keychain and
never in a project directory. For development only, copy the documented template:

```sh
cp .env.example .env
```

Fill only the providers you intend to exercise. Credential resolution is keychain, then environment,
then unconfigured; `.env` support is compiled out of release builds. ComfyUI uses
`http://127.0.0.1:8188` by default and can be changed in Settings. See
[Providers & BYOK](docs/08-providers.md) for capabilities and security details.

## Privacy and legal

Wobu operates no servers. There is no account, no inference proxy, no telemetry, no crash reporting
and no update check; the application never contacts us, because there is nowhere for it to contact.
Content leaves your machine only when you invoke a feature that uses a provider you configured, and
then it goes directly to that provider under your own credentials.

- [Privacy policy](docs/legal/privacy-policy.md) — every outbound destination and what is sent to
  it, what stays on disk and where, and how credentials are held in the OS keychain.
- [Terms of use and EULA](docs/legal/terms.md) — the MIT grant restated for a downloaded binary, the
  absence of warranty, and how provider terms and generated-content ownership pass through to your
  own agreement with each provider.

Both documents are shown in the app under Settings › Legal and ship beside the binary in every
installer, alongside `LICENSE` and `THIRD-PARTY-NOTICES.md`.

## Development

```sh
npm run check              # typecheck, lint, formatting check, frontend tests
npm run check:code-health  # dead-code and duplication gates
npm run build              # production frontend build

cd src-tauri
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Frontend tests use Vitest, Testing Library, and jsdom. Rust behavior is tested within the workspace
crates. Run a focused frontend test during iteration with `npm test -- path/to/file.test.tsx`, or a
focused Rust package with `cargo test -p wobu-store` from `src-tauri/`.

## Architecture

The UI uses React, TypeScript, Vite, Zustand, TanStack Query, and plain CSS. Tauri commands connect
it to a Rust workspace split by responsibility:

| Path | Responsibility |
| --- | --- |
| `src/` | UI components, client state, hooks, styles, and frontend tests |
| `src-tauri/src/` | Desktop shell, commands, application state, and provider orchestration |
| `src-tauri/crates/wobu-core` | Domain types, schemas, kinds, links, and validation |
| `src-tauri/crates/wobu-store` | Markdown/project I/O, assets, indexing, conflicts, and export |
| `src-tauri/crates/wobu-influence` | Influence resolution and prompt compilation |
| `src-tauri/crates/wobu-llm` | Text-provider adapters and streamed Enhance output |
| `src-tauri/crates/wobu-imagine` | Image and mesh backend adapters |
| `src-tauri/crates/wobu-jobs` | Queues, cancellation, progress, and retries |
| `src-tauri/crates/wobu-sync` | Peer identity, tickets, manifests, and blob transfer |

Start with the [design documentation index](docs/README.md) for the data model, architecture,
sharing guarantees, influence engine, provider behavior, acceptance evidence, and release process.

## Contributing

Read [AGENTS.md](AGENTS.md) for repository conventions and the checks expected before a pull
request. Keep changes focused, add regression coverage for behavioral fixes, update user-facing
documentation with feature changes, and never commit provider credentials or canonical user data.
