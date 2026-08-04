//! What is configurable, and — more to the point — what is not.
//!
//! Every default in this file is off. That is not a style choice: `Default` is
//! what a first launch, a corrupted settings file and a failed deserialise all
//! fall back to, so a `true` anywhere here would be a way for the server to end
//! up running without anybody having asked. The tests at the bottom pin it.
//!
//! There is no bind-address field. See `server.rs`.

use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq as _;

/// The port suggested on first run. `9628` is `WOBU` on a phone keypad, which
/// is the only claim being made for it: it is high, unassigned, and unlikely to
/// collide with the dev servers a person doing this work already has up.
pub const DEFAULT_PORT: u16 = 9628;

/// The bearer credential for the local endpoint.
///
/// A newtype rather than a `String` for two reasons. `Debug` prints a mask, so
/// a token cannot reach the diagnostics log through a `{:?}` somebody added in
/// passing — the same argument `redact.rs` makes in the shell. And comparison
/// only exists as [`Token::matches`], which is constant time, so there is no
/// `==` for a future call site to reach for.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Token(String);

impl Token {
    /// Thirty-two bytes from the platform's entropy source, as lowercase hex.
    ///
    /// Hex rather than base64 because this string is going to be pasted into a
    /// JSON config by hand, and a character set with no `+`, `/` or `=` in it
    /// survives that trip.
    pub fn generate() -> Token {
        use rand::Rng as _;
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let mut hex = String::with_capacity(64);
        for byte in bytes {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
        }
        Token(hex)
    }

    /// For a value read back out of the settings file, and for tests.
    pub fn from_raw(raw: impl Into<String>) -> Token {
        Token(raw.into())
    }

    /// The whole thing, for the one place that has to show it: the settings
    /// pane, when the user asks to see it so they can paste it into their agent.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Enough to tell two tokens apart on screen, and not enough to use.
    pub fn preview(&self) -> String {
        let head: String = self.0.chars().take(6).collect();
        format!("{head}…")
    }

    /// Constant time, and length-safe: `ct_eq` on slices of different lengths
    /// is `false` without a byte-wise comparison, so a short guess leaks
    /// nothing beyond being short.
    pub fn matches(&self, candidate: &str) -> bool {
        self.0.as_bytes().ct_eq(candidate.as_bytes()).into()
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(…)")
    }
}

/// The server half: Wobu exposing the open project.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSettings {
    /// Nothing listens until this is true, and only a person can make it true.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
    /// The second opt-in. Independent of `enabled`, so a user who wants an
    /// agent that reads their world but cannot touch it gets exactly that,
    /// which is the common case.
    #[serde(default)]
    pub allow_writes: bool,
    /// Generated the first time the server is enabled and kept until the user
    /// rotates it. `None` before then, which is also what a settings file
    /// written by an older build looks like.
    #[serde(default)]
    pub token: Option<Token>,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl Default for ServerSettings {
    fn default() -> Self {
        ServerSettings { enabled: false, port: DEFAULT_PORT, allow_writes: false, token: None }
    }
}

/// One MCP server the user has told Wobu about.
///
/// Enabling one of these means launching a program the user named, with the
/// arguments they gave, as themselves. That is a large thing to do on somebody's
/// behalf, and the only reason it is here is that it is exactly what they asked
/// for — which is why `enabled` defaults to false even for a server that has
/// just been added.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientServer {
    pub id: String,
    pub name: String,
    /// The executable. Run through the OS's own resolution, not a shell — there
    /// is no `sh -c` anywhere in this crate, so a semicolon in a field is a
    /// semicolon rather than a second command.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment for the child. The parent's environment is inherited
    /// as well; a server that needs a key generally wants one variable added,
    /// not the whole environment replaced.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    #[serde(default)]
    pub enabled: bool,
}

/// The client half: Wobu consuming servers the user runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSettings {
    /// The master switch. With this off, no configured server is launched no
    /// matter what its own `enabled` says — so a user who wants everything to
    /// stop has one place to click rather than a list to walk.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub servers: Vec<ClientServer>,
}

impl ClientSettings {
    /// The servers that may actually be launched right now: the master switch
    /// and the per-server switch, both.
    pub fn active(&self) -> impl Iterator<Item = &ClientServer> {
        let enabled = self.enabled;
        self.servers.iter().filter(move |server| enabled && server.enabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_is_off() {
        // The one test in this crate that would matter even if everything else
        // were deleted. `Default` is what an absent or unreadable settings file
        // resolves to.
        let server = ServerSettings::default();
        assert!(!server.enabled);
        assert!(!server.allow_writes);
        assert!(server.token.is_none());

        let client = ClientSettings::default();
        assert!(!client.enabled);
        assert!(client.servers.is_empty());
        assert!(!ClientServer::default().enabled);
    }

    #[test]
    fn a_settings_file_missing_every_field_still_deserialises_to_off() {
        // A half-written file, or one from a build before this feature existed.
        // Serde's `default` on each field is what carries this, and a field
        // added later without one would fail the parse and — worse — could take
        // a `#[serde(default)]` that was `true`.
        let server: ServerSettings = serde_json::from_str("{}").unwrap();
        assert!(!server.enabled);
        assert!(!server.allow_writes);
        assert_eq!(server.port, DEFAULT_PORT);

        let client: ClientSettings = serde_json::from_str("{}").unwrap();
        assert!(!client.enabled);
    }

    #[test]
    fn a_server_the_user_has_not_ticked_is_never_active_and_nor_is_any_of_them_with_the_master_off()
    {
        let one = ClientServer { id: "a".into(), enabled: true, ..ClientServer::default() };
        let two = ClientServer { id: "b".into(), enabled: false, ..ClientServer::default() };

        let off = ClientSettings { enabled: false, servers: vec![one.clone(), two.clone()] };
        assert_eq!(off.active().count(), 0);

        let on = ClientSettings { enabled: true, servers: vec![one, two] };
        assert_eq!(on.active().map(|server| server.id.as_str()).collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn a_token_never_prints_itself() {
        let token = Token::from_raw("sekritsekritsekrit");
        assert!(!format!("{token:?}").contains("sekrit"));
        // And a settings struct that contains one does not either — this is the
        // shape that ends up in a `diag::debug` of the whole config.
        let settings = ServerSettings { token: Some(token), ..ServerSettings::default() };
        assert!(!format!("{settings:?}").contains("sekrit"), "{settings:?}");
    }

    #[test]
    fn two_generated_tokens_differ_and_are_long_enough_to_be_worth_generating() {
        let one = Token::generate();
        let two = Token::generate();
        assert_eq!(one.expose().len(), 64);
        assert!(one.expose().chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(one.expose(), two.expose());
        assert!(one.matches(one.expose()));
        assert!(!one.matches(two.expose()));
        // Six characters and an ellipsis: enough to recognise, useless to use.
        assert_eq!(one.preview().len(), 6 + '…'.len_utf8());
    }
}
