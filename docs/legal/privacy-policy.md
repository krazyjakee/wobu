# Wobu Privacy Policy

**Last updated:** 2 September 2026 · Applies to Wobu 0.1.10 and later.

Wobu is a local-first desktop application published by Jake Cattrall ("we", "us"). It is
distributed as a binary and as source under the MIT licence.

This policy describes exactly what Wobu does with your data. It is written against the source
code, not against an intention: every destination named below corresponds to a literal address in
the repository, and every store named below corresponds to a path the application actually writes.

## Summary

- **We operate no servers.** There is no Wobu account, no Wobu inference proxy, no Wobu sync
  service and no Wobu storage. We never receive your worlds, your prompts, your images or your
  keys, because there is nowhere for them to arrive.
- **There is no telemetry.** No analytics, no usage metrics, no crash or error reporting, no
  version or update check, no "phone home" of any kind. The application does not contact us, and
  contains no code or dependency that could.
- **Your project stays in your project folder.** Notes stay as readable Markdown, assets stay as
  files, and the derived search index stays outside the project on your own machine.
- **Content leaves your machine only when you ask it to**, and only to a provider you configured
  yourself or to a peer you shared with yourself.
- **API keys prefer the operating-system keychain.** If it cannot answer, Wobu uses an owner-only
  file in its application-data directory. Keys never enter a project folder or travel back to the
  interface.
- **Nothing listens for connections, and no other program is run, unless you switch it on.** Agent
  access over MCP is off by default, binds only to your own machine when enabled, and needs a further
  separate switch before an agent can change anything. See section 3.5 — and note that an MCP server
  you configure yourself is a program of someone else's, which this policy cannot speak for.

## 1. Data controller and contact

Jake Cattrall. Contact via the project repository:
<https://github.com/krazyjakee/wobu>. Because we hold no data about you, there is nothing for us to
disclose, correct, export or erase on request — the equivalent actions are all performed by you, on
your own filesystem, as described in section 7.

## 2. What stays on your machine

### 2.1 Project folders

A Wobu project is an ordinary, self-contained `<Name>.wobu` directory that you choose the location
of. Wobu reads and writes it in place, and never copies it anywhere else. It contains:

| Path | Contents |
| --- | --- |
| `project.json` | Project metadata and the provider/model *selection* — never a credential |
| `nodes/<kind>/<slug>.md` | Your world's notes, as plain Markdown |
| `assets/originals/` | Imported and generated images and meshes |
| `.wobu/tmp` | Staging area for atomic writes |
| `.wobu/sessions` | Presence markers, so collaborators on a shared folder can see each other |
| `.wobu/accepting.json` | Marker written while the project is accepting a peer-to-peer share |

### 2.2 Application data

Machine-local data lives in the standard per-user application data directory — `~/.local/share/wobu`
on Linux, `~/Library/Application Support/wobu` on macOS, `%APPDATA%\wobu` on Windows:

| Path | Contents |
| --- | --- |
| `index/<project-id>.sqlite` | The SQLite search index. Entirely derived from your project folder; it holds no canonical data and deleting it is always safe |
| `recent.json` | The list of recently opened projects |
| `settings.json` | Per-installation settings, including your ComfyUI endpoint |
| `credentials/*.key` | Provider credentials used only when the OS credential store cannot answer; owner-only (`0600` files inside a `0700` directory on Unix, per-user AppData permissions on Windows) |
| `sync/shares.json` | Records of shares you created or accepted |
| `mesh-cache/` | Decoded 3D meshes, cached for display |
| `logs/wobu.log`, `logs/wobu.1.log` | Diagnostic log, two rolling files of 2 MiB each |
| `logs/level` | The diagnostic level you selected |

During a project transfer Wobu also writes a temporary database to your system temporary directory
(`wobu-transfer-<id>.sqlite`), which it removes when the transfer finishes.

### 2.3 The diagnostic log

The log is written only to the application data directory — never inside a project folder, so it
cannot be synced to a collaborator by accident. Every line passes through a redactor before it is
written, so credentials do not reach the file. It is never transmitted anywhere. If you want to
send it to us to accompany a bug report you must find it and attach it yourself; Settings ›
Diagnostics will reveal it in your file manager and show its contents.

## 3. What leaves your machine, and to whom

Wobu is "bring your own key" (BYOK). When you configure a provider, the application talks to that
provider **directly from your machine**, using your credentials, under your account. There is no
intermediary. Each of the following happens only when you have configured the provider in question
and then invoked the feature that uses it.

Requests honour your operating system's proxy configuration.

### 3.1 Text providers — the Enhance feature

| Destination | Operator | Sent |
| --- | --- | --- |
| `https://api.anthropic.com/v1/messages` | Anthropic PBC | The compiled prompt, the standing system instructions, the model id and a token limit, plus your Anthropic API key as an `x-api-key` header |
| `https://generativelanguage.googleapis.com/v1beta/interactions` | Google LLC | The same request shape, plus your Google AI API key |

The "compiled prompt" is the text you are working on together with whatever the influence stack
contributed to it — that is, other notes from your own project. These requests carry text only; no
images are attached.

### 3.2 Image and 3D providers

| Destination | Operator | Sent |
| --- | --- | --- |
| `https://generativelanguage.googleapis.com/v1beta` | Google LLC | `POST /interactions` to generate: the prompt, the negative prompt, generation parameters, and any reference images inline as base64. `GET /models/<model>` to check that a key works |
| `https://hunyuan.intl.tencentcloudapi.com` | Tencent Cloud | 3D mesh jobs: a text prompt, or a single reference image or a set of rendered turnaround views, base64-encoded, plus mesh parameters. Requests are signed with your Tencent SecretId/SecretKey (TC3-HMAC-SHA256); the secret itself is not transmitted |
| Tencent COS result URLs | Tencent Cloud | Nothing is uploaded. Wobu downloads the finished mesh and preview from the time-limited URLs the Hunyuan API returns. The host is whatever that response names |
| `http://127.0.0.1:8188` (default, editable) | You | ComfyUI: prompts, generation parameters and reference-image bytes over HTTP, plus a WebSocket to `…/ws` for progress |

Notes on the two that need them:

- **Tencent region.** Wobu picks between the `ap-singapore`, `na-siliconvalley` and `eu-frankfurt`
  endpoints using your computer's UTC offset, and you can change the choice in Settings. It does
  not perform a geo-IP lookup or read your location.
- **ComfyUI.** The default endpoint is your own loopback address, so by default nothing leaves the
  machine at all. If you change it to a LAN or internet address you are authorising Wobu to send
  prompts and reference-image bytes to that server, and its operator's policies then apply.

### 3.3 Peer-to-peer sync

Sync is off until you create or accept a share. When it is running, Wobu uses the `iroh` QUIC
transport to move project content directly between peers. Transfers are content-addressed and
verified with BLAKE3, and a share is authorised by a ticket you pass to the other person yourself —
there is no directory, index or rendezvous service that lists your projects.

Two pieces of third-party infrastructure are nevertheless involved, and we do not operate either of
them. Both are run by **Number Zero, Inc. (n0)**, the authors of `iroh`:

| Destination | Purpose |
| --- | --- |
| `https://dns.iroh.link/pkarr` and DNS queries under `dns.iroh.link` | Address discovery. Your machine **publishes** its endpoint public key, its relay URL and its direct IP addresses here so a peer holding your ticket can find you, and resolves the same records for peers |
| `use1-1.relay.n0.iroh.link`, `usw1-1.relay.n0.iroh.link`, `euc1-1.relay.n0.iroh.link`, `aps1-1.relay.n0.iroh.link` | Relays, over HTTPS on port 443 plus QUIC address discovery on port 7842. Used to make the initial connection through NAT, and to carry traffic if a direct path cannot be established |

Wobu selects the nearest relay by latency. Connections upgrade to a direct peer-to-peer path when
one is available, but a relay may carry project content for the whole session if it is not. If you
do not want your addresses published to, or your content routed through, infrastructure operated by
a third party, do not use peer-to-peer sync; shared-folder collaboration over a filesystem you
control has no such dependency.

### 3.4 Links you click

Buttons in Settings open pages in your normal web browser rather than fetching them:
`https://console.tencentcloud.com/hunyuan`, `https://console.tencentcloud.com/cam`,
`https://console.tencentcloud.com/cam/capi` and `https://github.com/krazyjakee/wobu`. Once your
browser is open, its own privacy behaviour applies.

Wobu also prints `https://aistudio.google.com/apikey` in the error it shows when Google reports a
billing problem with your key. That address is text on screen for you to follow if you want to; the
application never requests it.

### 3.5 Agent access (MCP) — off unless you turn it on

Wobu can expose your open project to an AI agent over the Model Context Protocol, and can run MCP
servers you configure. **All of it is off by default**, and nothing here starts until you turn it on
in Settings → Agent access. This is the only part of Wobu that listens for connections or launches
another program, so it is described in full:

- **A local listener.** When you enable the server, Wobu binds `127.0.0.1:9628` (the port is
  configurable; the address is not). It accepts connections only from your own machine. Requests must
  carry a bearer token that is generated the first time you enable it; any request arriving with an
  `Origin` header is refused before authentication, and no CORS header is ever sent, so a web page you
  visit cannot reach it. Nothing is sent anywhere by the server — it answers, it does not call out.
- **What an agent can read.** Nine read-only tools covering your entities, search, links, the
  resolved influence stack, compiled prompts and generation receipts. That is your project content,
  disclosed to whichever agent you connected.
- **What an agent can change** is a second, separate switch, also off by default. Until you grant it,
  the write tools are not merely refused — they are not advertised at all.
- **Programs Wobu runs.** If you configure MCP servers of your own, Wobu launches them directly (never
  through a shell), talks to them over stdin/stdout, and stops them when you disable them or quit. A
  newly added server is added switched off. What those programs do, and where they send anything, is
  governed by whoever wrote them, not by this policy.
- **What is stored.** `mcp.json` in the application data folder, readable only by your user account,
  holds the bearer token and any environment overrides you set for a configured server.
- **A record of use.** Every call, including refused ones, is timestamped into an in-app activity
  list and the local diagnostic log. That record stays on your machine.

Turning any switch off takes effect at once: the socket is dropped and configured servers are killed,
rather than a flag being consulted on the next request.

### 3.6 Providers' own handling of what you send

Once your content reaches a provider, that provider's privacy policy and terms — not this one —
govern what happens to it, including whether it may be retained, reviewed by humans, or used for
model training. Those decisions belong to your account with them. Please read them:

- Anthropic — <https://www.anthropic.com/legal/privacy>
- Google AI — <https://policies.google.com/privacy>
- Tencent Cloud — <https://www.tencentcloud.com/document/product/301/17345>
- ComfyUI — self-hosted; the operator of the endpoint you configured is responsible
- n0 (`iroh` relays and DNS) — <https://n0.computer/privacy>

## 4. What is never collected

We want this to be unambiguous. Wobu contains **no**:

- analytics or product-usage tracking of any kind;
- crash, error or exception reporting;
- performance or metrics collection, and no OpenTelemetry or similar exporter;
- automatic update check, version ping or licence check — the application never contacts a Wobu
  endpoint, because none exists;
- advertising, advertising identifier, cookie, or fingerprinting;
- account, sign-in, activation or registration;
- remote fonts, scripts, stylesheets or images. The interface loads nothing from a CDN.

We collect no personal data, so we do not sell, share or disclose personal data, and we do not
transfer it internationally. Any international transfer that occurs is one you initiate, directly
between your machine and a provider you chose.

The application's webview is additionally locked down by a content security policy that permits no
external origin, so the interface itself cannot make a network request off the device. All provider
traffic is made by the native process, from the code paths listed in section 3.

## 5. How API keys are stored

Credentials you enter are stored per installation. Wobu first tries the operating system's own
credential store — Keychain on macOS, Credential Manager on Windows, and the Secret Service (GNOME
Keyring, KWallet) on Linux — under the service name `wobu`:

| Entry | Credential |
| --- | --- |
| `wobu/anthropic` | Anthropic API key |
| `wobu/gemini` | Google AI API key |
| `wobu/tencent-secret-id` | Tencent SecretId |
| `wobu/tencent-secret-key` | Tencent SecretKey |
| `wobu/sync` | Your peer identity secret key, generated locally |

ComfyUI has no entry because it needs no key.

If the OS store refuses or does not answer within half a second, Wobu stores the provider key as
plain text under `credentials/<provider>.key` in the application-data directory described in
section 2.2. On Unix the directory is mode `0700` and each key file is mode `0600`; on Windows the
file inherits the per-user AppData ACL. This fallback is not encrypted independently of the user
account. It exists so a locked or missing Linux Secret Service cannot make key entry impossible.
Settings identifies a key held there as “in Wobu's private local store.” Once a fallback exists,
Wobu reads it first and does not retry the unresponsive OS service during Enhance or Generate.

Consequences of that design, all of them deliberate:

- **Keys are never written into a project folder.** A key in `project.json` would be a key handed
  to everyone the folder is shared with. The project file records only the *selection* — provider
  id, model id, default parameters.
- **Keys never travel back to the interface.** A key passes from the entry field into the native
  process once. After that the interface can ask whether a provider is configured, but cannot read
  the value.
- **Keys are excluded from logs and error output.** The credential type has no serialisation and
  prints as a mask; provider adapters print `<redacted>` in place of the key.
- **Release builds never read credentials from a project or arbitrary file.** The private local
  fallback has one fixed application-data path. Development builds may additionally read a key
  from an environment variable or repo-root `.env`; that mechanism is compiled out of releases.
- **If the keychain is unavailable** — a locked login keyring, a headless session — Add, Replace,
  Save and Remove remain usable through the private local fallback.

You can delete any credential from Settings › Providers. Wobu removes the local entry and asks the
OS store to remove its entry; if that service is still unavailable, a local tombstone prevents an
older native value from becoming active later.

## 6. Children

Wobu is not directed at children and collects no data from anyone, including children. Providers
you configure may impose their own minimum-age requirements.

## 7. Your control over your data

Because everything is local, you exercise your rights directly rather than by asking us:

- **Access and portability** — your world is Markdown and ordinary image files in a folder you
  chose. Copy it, back it up, put it in version control, or open it in any other editor.
- **Erasure** — delete the project folder. To remove everything else, delete the application data
  directory listed in section 2.2; the search index and mesh cache are derived and safe to delete at
  any time.
- **Withdrawing a provider** — delete its key in Settings › Providers and no further request can be
  made to it. Content already sent to a provider is subject to that provider's retention policy, and
  must be deleted through them.
- **Stopping sync** — stop sharing, and Wobu stops publishing addresses and stops connecting to
  relays.

## 8. Security

Wobu prefers the OS keychain and otherwise restricts fallback credential files to the current OS
user, redacts keys from logs and error paths, writes project files atomically through a staging
directory, and verifies peer-to-peer transfers cryptographically.
It cannot, however, protect data on a machine that is itself compromised, and current beta bundles
are unsigned — download them only from the project's GitHub Releases page.

## 9. Changes to this policy

The current version of this policy is the one in the release you are running; it ships beside the
binary in the installer and is shown in Settings › Legal. When the set of destinations in section 3
changes, this document changes with it in the same release. The history of every change is public in
the repository.

## 10. Complaints

If you believe Wobu is sending data somewhere this document does not list, please open an issue at
<https://github.com/krazyjakee/wobu/issues>. If you are in the UK or the EU and are dissatisfied
with our response, you may complain to your national data protection authority — in the UK, the
Information Commissioner's Office at <https://ico.org.uk>.
