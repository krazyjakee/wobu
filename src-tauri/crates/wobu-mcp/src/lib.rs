//! Model Context Protocol, both directions, and off in both directions until
//! somebody says otherwise.
//!
//! Wobu holds a structured world — nodes, links, the resolved influence stack,
//! the compiled prompt, and a receipt for every generation that was ever paid
//! for. All of that is useful to an agent, and none of it is anybody's business
//! but the user's. So this crate is built the other way round from most protocol
//! crates: the interesting part is not the wire format, it is the set of things
//! that have to be true before a byte moves at all.
//!
//! ## The rules this crate exists to keep
//!
//! 1. **Nothing listens and nothing is spawned by default.** [`server::Server`]
//!    is only ever constructed by a caller that has been handed an explicit
//!    `enabled` flag, and [`client::Registry`] only spawns a process for a
//!    server the user typed in and then separately ticked.
//! 2. **The listener is loopback and cannot be talked out of it.**
//!    [`server::Server::start`] binds `127.0.0.1` — the address is not a
//!    parameter, only the port is. There is no configuration in this crate, in
//!    the shell, or in a settings file that makes it bind `0.0.0.0`.
//! 3. **Every request carries a bearer token or is refused.** Loopback is not a
//!    trust boundary on a desktop: every process on the machine can reach it,
//!    and so can a page in a browser. The token is compared in constant time
//!    ([`server`]), and a request with an `Origin` header — which is to say, a
//!    request from a web page — is refused before the token is even looked at.
//! 4. **Reads and writes are separate opt-ins.** The write tools are not
//!    merely refused when writes are off; they are not *advertised*, so an
//!    agent that asks what it can do is told the truth. See [`dispatch`].
//! 5. **Every call is reported.** [`Audit`] is not optional plumbing — the
//!    dispatcher takes one and calls it for every tool invocation, so the
//!    person who opened the door can see what came through it.
//!
//! ## What this crate is not
//!
//! It is not a complete MCP implementation and does not pretend to be. There is
//! no SSE stream, no sampling, no prompt registry, no server-initiated
//! anything. What it speaks is the subset a coding or writing agent actually
//! uses — `initialize`, `tools/list`, `tools/call`, `resources/list`,
//! `resources/read`, `ping` — over one POST per request. A feature that is
//! absent is easier to reason about than a feature that is present and stubbed,
//! and the whole argument for this crate rests on being able to reason about it.
//!
//! It also holds no domain knowledge. The tools are named and described here
//! and executed through the [`World`] trait, which the shell
//! implements against the open project. That split is what lets the tests below
//! exercise the whole protocol without a Tauri app, a project folder or a disk.

pub mod client;
pub mod config;
pub mod dispatch;
pub mod jsonrpc;
pub mod server;
pub mod tools;
pub mod world;

pub use config::{ClientServer, ClientSettings, ServerSettings, Token};
pub use dispatch::{Audit, CallRecord, Dispatcher};
pub use server::{Running, Server};
pub use tools::{Tool, catalogue};
pub use world::{NodePatch, World, WorldError};

use std::net::SocketAddr;

/// Everything that can go wrong on this crate's own terms.
///
/// Deliberately small. A tool that fails because the project is closed is not
/// an error of *this* crate — it is an answer, and it travels back to the agent
/// inside a `tools/call` result as [`WorldError`]. What is
/// here is the handful of failures that mean the door itself would not open.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The port is taken. Almost always a second Wobu, or a previous one that
    /// has not finished exiting; worth saying so rather than reporting a bare
    /// `AddrInUse` that sends the user looking for a firewall.
    #[error("nothing could listen on 127.0.0.1:{port} — something else already is")]
    PortInUse { port: u16 },
    #[error("could not listen on {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    /// A configured MCP server would not start, would not answer, or answered
    /// with something that is not MCP. The string is the user's to read.
    #[error("{0}")]
    Client(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
