# 10 — Sync spike: what iroh 1.0 actually does

[09](09-roadmap.md)'s M3 estimates every issue against an API nobody here had called. This is
the note that closes that gap ([#74](https://github.com/krazyjakee/wobu/issues/74)). It is a
record of what was **compiled and run**, not a design and not a summary of the docs.

> ✅ **Verified by compiling and running, 2026-08-01.** Everything under "Ran" below came out
> of a two-process demo built against the real crates on this machine. Everything under
> "Read" was taken from the vendored source of `iroh 1.0.3` in the cargo registry, not from
> memory or from docs.rs. Claims that could not be tested here are marked 🚩 and say why.

**The demo lives outside this repo, on purpose.** It is a standalone cargo project in the
session scratchpad (`scratchpad/iroh-spike/`), so `iroh` never entered `src-tauri/Cargo.lock`.
Adding a 156-crate dependency tree to answer a question is how a spike turns into a
commitment. Nothing in `src-tauri/` was touched.

---

## The premise that was already stale

The issue says:

> Note the tokio requirement. The workspace is entirely synchronous today — no `async fn`
> anywhere — so this is the first real runtime.

**That is no longer true**, and it matters, because "introduce async to a sync codebase" was
carrying weight in M3's sizing. As of `src-tauri/Cargo.lock` today:

| | |
| --- | --- |
| `async fn` in the workspace | **75** — `wobu-imagine` 31, `wobu-jobs` 28, `wobu-llm` 10, the Tauri shell 6 |
| `tokio` | **1.53.1**, exactly one copy in the lock |
| Also present | `async-trait` 0.1.91, `reqwest` 0.13.4, `tokio-tungstenite` 0.30.0, `rustls` 0.23.43, `blake3` 1.8.5, `quinn` 0.11.11 (via `reqwest`) |

`wobu-jobs` ([#49](https://github.com/krazyjakee/wobu/issues/49)) already owns a multi-thread
runtime, `wobu-imagine` ([#51](https://github.com/krazyjakee/wobu/issues/51)) already holds a
websocket open across it, and Tauri brings its own. M3 is not the first runtime; it is
another consumer of one that exists.

### Would iroh bring a second tokio? No.

This was the finding worth checking, and the answer is clean:

- `iroh 1.0.3` requires `tokio ^1.44.1` (and `^1` for the wasm target). The workspace's
  **1.53.1 satisfies it**, so the lock unifies on one tokio. The spike's own lock resolved
  `tokio 1.53.1` — the same version, independently.
- `reqwest 0.13.4`, `rustls 0.23.43` and `blake3 1.8.5` **also unify**: iroh wants
  `reqwest ^0.13`, `rustls ^0.23.33`, `blake3 ^1.8.3` and the workspace is already above each.
- 🚩 One thing to know before reading old iroh material: **iroh 1.0 does not use `quinn`.**
  It depends on `noq` (`^1.1.0`), n0's own QUIC stack, resolving to `noq 1.1.1`. So the
  `quinn 0.11.11` already in the lock via `reqwest` is untouched and unrelated — there is no
  duplicate-QUIC problem, but there is also no shared QUIC layer to reason about.

Diffing the spike's lock against the workspace's: **156 crates would be new**, plus
duplicate-major copies of `sha2`/`digest` (iroh is on 0.11, the workspace on 0.10) which
coexist harmlessly. No shared crate is forced to a version the workspace cannot take.

---

## Version pairing

`iroh` is on 1.x; `iroh-blobs` is on its own 0.10x line and pins iroh **exactly** during
release candidates. This is the pairing table.

| `iroh` | `iroh-blobs` | `iroh-docs` | `iroh-gossip` | Result |
| --- | --- | --- | --- | --- |
| **1.0.3** | **0.103.0** | **0.101.0** | **0.101.0** | ✅ **compiles and runs** — this is the set to use |
| 1.0.3 | 0.102.0 | — | — | ❌ resolver error, `iroh-blobs 0.102` pins `iroh =1.0.0-rc.1` |
| 1.0.3 | 0.100.0 | — | — | ❌ resolver error via `ed25519-dalek` |
| 1.0.0 – 1.0.2 | 0.103.0 | 0.101.0 | 0.101.0 | ⚠️ should work (`iroh-blobs 0.103` asks for `iroh ^1.0.0`) — not tested |

Release dates, from the crates.io API: `iroh` 1.0.0 on **2026-06-15**, 1.0.1 on 06-29,
1.0.2 on 07-06, 1.0.3 on **2026-07-20**. `iroh-blobs` 0.103.0, `iroh-docs` 0.101.0 and
`iroh-gossip` 0.101.0 all landed together on **2026-06-15**, the same day as iroh 1.0.0.
The satellite crates are cut in one batch against each iroh release.

### What a mismatch looks like

**Good news, and worth saying plainly: you cannot silently end up with two irohs.** Both
mismatch modes fail at *resolve* time, before any compilation, because `iroh-blobs` pins
`iroh-base` and transitively `ed25519-dalek` with `=` requirements. Verbatim:

```
$ cargo generate-lockfile          # iroh = "1.0.3", iroh-blobs = "0.102.0"
error: failed to select a version for `iroh`.
    ... required by package `iroh-blobs v0.102.0`
versions that meet the requirements `=1.0.0-rc.1` are: 1.0.0-rc.1
all possible versions conflict with previously selected packages
  previously selected package `iroh v1.0.3`
failed to select a version for `iroh` which could resolve this conflict
```

```
$ cargo generate-lockfile          # iroh = "1.0.3", iroh-blobs = "0.100.0"
error: failed to select a version for `ed25519-dalek`.
    ... required by package `iroh v1.0.3`
versions that meet the requirements `>=3.0.0-rc.0, <4.0.0` are: 3.0.0, 3.0.0-rc.1, 3.0.0-rc.0
  previously selected package `ed25519-dalek v3.0.0-pre.6`
    ... which satisfies dependency `iroh-base = "^0.98"` of package `iroh-blobs v0.100.0`
```

The second one is the confusing one — the error names `ed25519-dalek`, which you did not ask
for and have never heard of. **If you see an `ed25519-dalek` resolver conflict, the real
problem is that your `iroh-blobs` and `iroh` are from different generations.**

---

## The four API shapes

Signatures copied out of the vendored source at
`~/.cargo/registry/src/index.crates.io-*/iroh-1.0.3/`. The demo compiles against all of them.

### 🚩 The rename that will bite the whole milestone

**`NodeId` and `NodeAddr` do not exist in iroh 1.0.** They are `EndpointId` and `EndpointAddr`:

```rust
// iroh-base-1.0.3/src/key.rs:70
pub type EndpointId = PublicKey;              // 32-byte ed25519 public key

// iroh-base-1.0.3/src/endpoint_addr.rs
pub struct EndpointAddr {
    pub id: EndpointId,
    pub addrs: BTreeSet<TransportAddr>,
}

pub enum TransportAddr {
    Relay(RelayUrl),
    Ip(SocketAddr),
    Custom(CustomAddr),
}
```

[#76](https://github.com/krazyjakee/wobu/issues/76) is titled "replace `$USER` with an iroh
`NodeId`". The type it means is `EndpointId`. Every M3 issue body and every roadmap mention
of "node id" needs reading as "endpoint id", and any sample code found online older than
2026-05 will not compile.

### 1. Binding an `Endpoint`

```rust
impl Endpoint {
    pub fn builder(preset: impl Preset) -> Builder;
    pub async fn bind(preset: impl Preset) -> Result<Self, BindError>;
    pub fn id(&self) -> EndpointId;
    pub fn addr(&self) -> EndpointAddr;
    pub async fn online(&self);
    pub async fn close(&self);
}

impl Builder {
    pub fn alpns(mut self, alpn_protocols: Vec<Vec<u8>>) -> Self;
    pub fn secret_key(mut self, secret_key: SecretKey) -> Self;
    pub fn relay_mode(mut self, relay_mode: RelayMode) -> Self;
    pub fn clear_ip_transports(mut self) -> Self;
    pub fn hooks(mut self, hooks: impl EndpointHooks + 'static) -> Self;
    pub async fn bind(self) -> Result<Endpoint, BindError>;
}
```

`Preset` is new in 1.0 and is **mandatory** — `presets::N0` (n0 relays + DNS/pkarr address
lookup), `presets::Minimal` (crypto provider only), `presets::Empty` (fails to bind; for
callers configuring everything). `Endpoint::bind(presets::Empty)` is a compile-time-legal,
runtime-guaranteed failure, which is a trap worth knowing about.

**`online().await` matters.** Before it resolves, `addr()` may have no relay URL, so a ticket
minted too early is not dialable from outside the LAN. In the demo the accept side awaits
`online()` before writing the ticket.

### 2. Registering a custom ALPN

Two ALPNs on one endpoint, via the `Router`, is what M3 needs — wobu's own protocol *and*
`iroh-blobs` on the same connection stack:

```rust
impl Router {
    pub fn builder(endpoint: Endpoint) -> RouterBuilder;
    pub fn endpoint(&self) -> &Endpoint;
    pub async fn shutdown(&self) -> Result<(), n0_future::task::JoinError>;
}

impl RouterBuilder {
    pub fn accept(
        mut self,
        alpn: impl AsRef<[u8]>,
        handler: impl Into<Box<dyn DynProtocolHandler>>,
    ) -> Self;
    #[must_use]
    pub fn spawn(self) -> Router;
}
```

```rust
let router = Router::builder(endpoint)
    .accept(b"wobu/sync/1", SyncProto { manifest })
    .accept(iroh_blobs::ALPN, blobs)      // iroh_blobs::ALPN == b"/iroh-bytes/4"
    .spawn();
```

`spawn()` sets the endpoint's ALPN list from the registered handlers, so `Builder::alpns` is
not needed when using a `Router`. `Router` **aborts when dropped** — it is `#[must_use]` and
the handle must be held for the lifetime of the service. That is a real constraint on
[#82 `SyncManager`](https://github.com/krazyjakee/wobu/issues/82).

### 3. Accepting a connection

```rust
pub trait ProtocolHandler: Send + Sync + std::fmt::Debug + 'static {
    fn on_accepting(&self, accepting: Accepting)
        -> impl Future<Output = Result<Connection, AcceptError>> + Send;   // has a default

    fn accept(&self, connection: Connection)
        -> impl Future<Output = Result<(), AcceptError>> + Send;

    fn shutdown(&self) -> impl Future<Output = ()> + Send;                 // has a default
}
```

Note it is an **RPITIT trait, not `async_trait`** — no `#[async_trait]` attribute, and the
impl is written as a plain `async fn accept(&self, connection: Connection)`. It also demands
`Debug`, which is easy to miss. `AcceptError::from_err` wraps foreign errors.

The returned future runs on its own spawned task and may be long-lived; when it returns the
`Connection` is dropped. Raw `Endpoint::accept()` is also available for a hand-rolled loop:

```rust
pub fn accept(&self) -> Accept<'_>;      // yields Option<Incoming>; None once closed
```

### 4. Dialing by endpoint id

```rust
pub async fn connect(
    &self,
    endpoint_addr: impl Into<EndpointAddr>,
    alpn: &[u8],
) -> Result<Connection, ConnectError>;
```

`impl Into<EndpointAddr>` is the important part: `From<EndpointId> for EndpointAddr` exists
and produces an address with **no** network paths, which forces the configured address-lookup
service to resolve it. So `endpoint.connect(some_endpoint_id, ALPN)` is literally the
dial-by-id case, and the demo exercises it (`dial --id-only`).

### Blobs, for completeness

```rust
FsStore::load(root: impl AsRef<Path>) -> Result<Self>           // async
store.blobs().add_path(path: impl AsRef<Path>) -> AddProgress   // await -> TempTag {hash, format}
store.blobs().export(hash, target) -> ExportProgress
BlobsProtocol::new(store: &Store, events: Option<EventSender>) -> Self
api.downloader(&endpoint) -> Downloader
downloader.download(request: impl SupportedRequest, providers: impl ContentDiscovery)
    -> DownloadProgress
```

`download(hash, Some(endpoint_id))` works because `Hash: SupportedRequest` and
`Option<EndpointId>: ContentDiscovery`. The progress types are futures *and* streams, so
`.await` gives completion and polling gives progress — which is what
[#81](https://github.com/krazyjakee/wobu/issues/81) and the job queue want.

---

## Relay fallback — the answer to "not hope"

**Yes, the caller can tell direct from relayed, per path, at any moment.**

```rust
impl Connection {
    pub fn paths(&self) -> PathList<'_>;              // snapshot at call time
    pub fn paths_stream(&self) -> PathListStream<'_>; // stream of snapshots
    pub fn path_events(&self) -> PathEventStream;     // individual events, 'static
}

impl<'conn> Path<'conn> {
    pub fn id(&self) -> PathId;
    pub fn remote_addr(&self) -> &TransportAddr;
    pub fn is_selected(&self) -> bool;   // currently carrying application data
    pub fn is_ip(&self) -> bool;         // direct, hole-punched
    pub fn is_relay(&self) -> bool;      // via a relay server
    pub fn stats(&self) -> PathStats;
    pub fn rtt(&self) -> Duration;
}

pub enum PathEvent {                     // #[non_exhaustive]
    Opened   { id, remote_addr, local_addr },
    Closed   { id, remote_addr, local_addr, last_stats },
    Selected { id, remote_addr, local_addr },   // this path is now carrying data
    Lagged   { missed },                        // consumer fell behind
}
```

The model is **not** "direct or relayed". A connection holds *several* paths at once and one
is `is_selected()`. Relay is not a fallback that replaces the direct path; it is a path that
comes up first and stays open, unselected, once holepunching wins.

### Ran: the relay → direct upgrade, observed

Dialing the bare `EndpointId` (`--id-only`), the connection came up **relayed** and upgraded
in place. Real output, one process to another:

```
[dial] ID-ONLY: discarding addrs, dialling bare EndpointId
[dial] connected in 165.549658ms to 14ca40b9ff
[dial] 1 open path(s):
[dial]   id=PathId(0) RELAY  selected=true  rtt=65.367389ms remote=relay:https://euc1-1.relay.n0.iroh.link./
[dial] manifest: [("character/kael-vantris", "b467afa3…"), ("location/ashfall", "deadbeef")]
[dial] paths changed -> RELAY DIRECT*
[dial] 2 open path(s):
[dial]   id=PathId(0) RELAY  selected=false rtt=65.455312ms remote=relay:https://euc1-1.relay.n0.iroh.link./
[dial]   id=PathId(1) DIRECT selected=true  rtt=1.008698ms  remote=ip:172.17.0.1:37553
[dial] downloaded b467afa3… in 20.54985ms
[dial] VERIFIED 524288 bytes
```

So: **the connection is usable before holepunching finishes.** Application data flowed over
the relay path while the direct path was still being negotiated. Nothing has to wait, and
nothing has to be retried. `PathEvent::Selected` is the signal that the upgrade happened.

### Ran: forced relay-only, the pair that never holepunches

`Builder::clear_ip_transports()` removes every UDP transport, leaving only the relay. This is
the closest thing to a hostile-NAT simulation available on one machine, and it is how the
"what happens then" question got an answer rather than a hope:

```
[accept] RELAY-ONLY: ip transports cleared
[accept] addr = EndpointAddr { id: …, addrs: {Relay(https://euc1-1.relay.n0.iroh.link./)} }
[dial]   id=PathId(0) RELAY selected=true rtt=63.547443ms
[dial] downloaded b467afa3… in 504.206792ms
[dial] VERIFIED 524288 bytes, blake3 b467afa3…
```

**It works. It is just slower.** Same 512 KiB blob:

| | connect | RTT | 512 KiB blob transfer |
| --- | --- | --- | --- |
| Direct (LAN) | 15 ms | ~1–2 ms | **22 ms** |
| Relay only | 74 ms | ~65 ms | **504 ms** |

≈23× slower, over an n0 relay in `euc1` from the UK. There is no failure mode to design for
here — there is a **performance** mode to design for, and a UI that should say "connected via
relay" rather than pretending everything is fine. That is a smaller problem than M3 assumed.

### 🚩 What this machine could *not* test

**A two-process demo on one host does not exercise NAT.** Both endpoints shared a public IP,
a LAN and a loopback, so "direct" here is trivially easy and the holepunch that succeeded
proves nothing about a symmetric-NAT pair. Specifically still unknown:

- 🚩 How long a real holepunch takes when it works, across two consumer routers.
- 🚩 What fraction of user pairs never get a direct path. iroh publishes no number we
  verified, and we have no measurement.
- 🚩 Whether relay throughput holds up for a project with hundreds of MB of assets. 504 ms
  for 512 KiB extrapolates to ~1 MB/s, which would be ~17 minutes for a 1 GB project — but a
  single measurement of a single blob is not a throughput benchmark.
- 🚩 What happens when the relay itself is unreachable (corporate proxy, blocked 443).
  `RelayMode::Disabled` exists; the failure text was not captured.
- 🚩 **We would be using n0's relays**, which contradicts "no relay we operate" in [09](09-roadmap.md)
  only in spirit — we operate none, but we would depend on infrastructure someone else
  operates for free. That is a posture decision, not a technical one, and it is not made yet.

---

## Is `iroh-docs` maintained enough to lean on?

**The assumption going in was "no". The evidence says the assumption is wrong** — and this is
the more valuable answer, because it changes what M3 could be.

| Evidence | `iroh-docs` |
| --- | --- |
| Latest release | 0.101.0, **2026-06-15** — same day as iroh 1.0.0 |
| Tracks iroh 1.0? | **Yes**, `iroh ^1`, `iroh-blobs ^0.103`, `iroh-gossip ^0.101.0` |
| Last commit | **2026-07-30** (two days ago) |
| Archived / deprecated? | No |
| Release cadence | 0.97 → 0.101 across 2026-03, 04, 05, 05, 06 — **in lockstep with every iroh release** |
| Committers | `rklaehn`, `dignifiedquire`, `ramfox`, `Frando` — n0 core, not drive-by |
| Downloads | 100k lifetime, 10.3k on 0.101.0 |

For contrast, **`iroh-blobs` — which M3 already plans to depend on — was last pushed
2026-07-20, ten days *before* `iroh-docs`.** By the "recently touched" test, docs is the
better-maintained of the two.

I confirmed this by compiling, not by reading badges. A four-crate project resolved and built
cleanly and the protocol spawned:

```
$ cargo build && ./docs-check
iroh-docs 0.101 + iroh 1.0.3 + iroh-blobs 0.103 built and spawned
```

against this exact API:

```rust
let gossip = Gossip::builder().spawn(endpoint.clone());
let docs   = Docs::memory().spawn(endpoint.clone(), (*store).clone(), gossip.clone()).await?;
let router = Router::builder(endpoint)
    .accept(BLOBS_ALPN, blobs)
    .accept(GOSSIP_ALPN, gossip)
    .accept(DOCS_ALPN,  docs)
    .spawn();
```

Cost: `iroh-docs` + `iroh-gossip` add only **~10 crates** on top of what `iroh` + `iroh-blobs`
already pull (441 vs 432 packages) — it is not a heavy addition.

### The honest caveats, which are about *correctness*, not abandonment

Maintained is not the same as trustworthy, and the open tracker is where the real risk is:

- **Unmerged fixes for live sync bugs.** [#112](https://github.com/n0-computer/iroh-docs/pull/112)
  "retry missing content downloads on every successful sync",
  [#111](https://github.com/n0-computer/iroh-docs/pull/111) "clear connect state when the
  remote aborts a dial as already-syncing", and
  [#114](https://github.com/n0-computer/iroh-docs/pull/114) "return the sync outcome from
  `BobState::run`" — all opened 2026-07-14…20, **all still open**. Those are exactly the
  paths M3 would live on.
- [#82](https://github.com/n0-computer/iroh-docs/issues/82) "Sync with other peers never
  re-downloads blobs", open since 2026-02.
- [#78](https://github.com/n0-computer/iroh-docs/issues/78) "Docs gets corrupted state when
  app terminated after `doc.set_bytes()` success", open since 2025-11. For wobu that is the
  one class of bug [07](07-file-shares.md) exists to prevent.
- [#49](https://github.com/n0-computer/iroh-docs/issues/49) "TODOs / Missing features in 0.9X
  vs iroh-docs 0.35" — the post-extraction rewrite dropped capability that has not returned.
- **It is a redb key-value store with its own on-disk format**, plus a second `redb_v3` for
  migration. Adopting it means a second source of truth next to the SQLite index and the
  Markdown files, and its replica model (namespace key = write capability, author key =
  authorship) is a genuinely different identity model from wobu's.

**Verdict: keep the roadmap's plan — `iroh` + `iroh-blobs`, own reconciliation — but for a
different reason than the one written down.** Not "iroh-docs is unmaintained" (false), but
"iroh-docs owns the disk format and the conflict semantics, and [09](09-roadmap.md)'s
three-way hash compare deliberately keeps the Markdown hand-editable and Obsidian-compatible".
Range-based set reconciliation is more machinery than a per-peer base hash needs. The
roadmap's 🚩 on this line should be replaced with that sentence, not with "unmaintained".

---

## What the demo did

`scratchpad/iroh-spike/` — two binaries, `accept` and `dial`, ~230 lines total.

`accept` binds an endpoint, awaits `online()`, loads an `FsStore`, imports a 512 KiB file,
spawns a `Router` carrying **both** `wobu/sync/1` and `iroh_blobs::ALPN`, and writes a
JSON ticket (`EndpointAddr` + blob hash + length). `dial` reads the ticket, binds its own
endpoint, connects on `wobu/sync/1`, does a toy node-id → hash manifest exchange over one
bidirectional stream ([#79](https://github.com/krazyjakee/wobu/issues/79) in miniature),
reports its paths, then fetches the blob with the blobs downloader, exports it, and asserts
the bytes match.

Three runs, two OS processes each, all green:

| Run | Path taken | Result |
| --- | --- | --- |
| default (full ticket) | direct immediately; relay path opened alongside | 512 KiB verified in 22 ms |
| `--relay-only` | relay only, both sides | 512 KiB verified in 504 ms |
| `--id-only` | relay first, upgraded to direct mid-connection | 512 KiB verified in 21 ms |

```
[accept] wobu/sync/1 connection from 4027e340a5
[accept] negotiated alpn = "wobu/sync/1"
[accept]   id=PathId(0) DIRECT selected=true rtt=11.421417ms remote=ip:192.168.0.129:54728
[accept] got request: {"want":"manifest"}
[accept] sent manifest (139 bytes)
[dial] VERIFIED 524288 bytes, blake3 b467afa334cd7dedd76e3a9a4b0e8a5ed278713f2f4e220a682aa30c7fcd140b
```

Build cost, measured on this machine (24 cores, `-j 8`, warm registry): **37 s** for a cold
debug build of all 432 crates, producing 2.8 GB of artifacts and a 350 MB debug binary.

---

## What this means for M3's estimates

| Issue | Was sized assuming | Actually |
| --- | --- | --- |
| [#75](https://github.com/krazyjakee/wobu/issues/75) `wobu-sync` endpoint + handshake `M` | first async runtime; unknown API | **smaller.** The runtime exists; the endpoint + custom-ALPN + `Router` shape is ~40 lines and is proven above. Closer to `S`/small-`M`. |
| [#76](https://github.com/krazyjakee/wobu/issues/76) peer identity `M` | `NodeId` | **rename first.** The type is `EndpointId = PublicKey` (32-byte ed25519). Size unchanged, but the issue text and every doc mention need correcting or the work starts from a name that does not exist. |
| [#77](https://github.com/krazyjakee/wobu/issues/77) tickets `M` | hand-rolled | **check `iroh-tickets` 1.0 first** — it is already in the tree via `iroh-blobs`, and `BlobTicket` shows the encoding pattern. Possibly `S`. |
| [#79](https://github.com/krazyjakee/wobu/issues/79) manifest exchange `M` | unknown | **unchanged.** One bidi stream, `write_all` / `read_to_end`, serde. The demo does it in 15 lines. |
| [#81](https://github.com/krazyjakee/wobu/issues/81) blob transfer `M` | unknown | **smaller.** `store.downloader(&ep).download(hash, Some(id))` plus `export`. The content-addressed, write-once posture of `assets/**` maps onto blobs exactly. Closer to `S`. |
| [#82](https://github.com/krazyjakee/wobu/issues/82) `SyncManager` `L` | unknown | **unchanged, and confirmed necessary.** `Router` aborts on drop, so something must own it for the process lifetime — which is the issue. |
| [#83](https://github.com/krazyjakee/wobu/issues/83) sync status / presence `M` | "hope" about relay | **now concrete.** `path_events()` gives `Opened`/`Closed`/`Selected`; `Path::is_relay()` gives the badge. The UI needs a *relayed* state, not just online/offline. |
| [#85](https://github.com/krazyjakee/wobu/issues/85) two-peer integration tests `L` | unknown | **feasible in-process.** Two `Endpoint`s in one tokio test work; `clear_ip_transports()` gives a deterministic relay-only case. But 🚩 those tests hit **n0's real relays** over the network unless a local relay is stood up (`iroh` has a `test-utils` feature) — CI needs deciding. |

Net: **M3 is smaller than it was sized**, mostly because the async foundation is already paid
for and the blobs half is nearly free. The remaining `L` is where it always was —
[#80 three-way apply](https://github.com/krazyjakee/wobu/issues/80) — and that is wobu's own
logic, which no amount of iroh confirms or reduces.

Two new pieces of work that were not on the list:

1. **`NodeId` → `EndpointId` across the roadmap and issue bodies**, before anyone writes code
   against a name that no longer exists.
2. **A decision about depending on n0's relay infrastructure**, which [09](09-roadmap.md)'s
   "no relay we operate" is silent about but which the fallback story requires.

---

## Not confirmed

- 🚩 Everything under "what this machine could not test" above — NAT traversal, real-world
  holepunch success rate, relay throughput at project scale, relay-blocked networks.
- 🚩 `iroh 1.0.0`–`1.0.2` paired with `iroh-blobs 0.103.0`. Only 1.0.3 was built.
- 🚩 Windows and macOS. Linux only.
- 🚩 Release-profile build cost and binary size impact on the shipped Tauri bundle. Only a
  debug build was measured.
- 🚩 `iroh-docs`' actual sync behaviour. It was compiled and spawned; **no document was ever
  synced**. The verdict above is about maintenance and fit, not about whether it works.
- 🚩 Whether iroh's `EndpointHooks` / `after_handshake` is the right place for wobu's
  authorisation check (who may sync this project). It exists and returns
  `AfterHandshakeOutcome::Accept`; it was read, not used.
