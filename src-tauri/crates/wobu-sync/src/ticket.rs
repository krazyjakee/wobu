//! One string that says: this project, that peer, and you were invited.
//!
//! The whole sharing UX of [#77](https://github.com/krazyjakee/wobu/issues/77)
//! is a token somebody pastes into a chat window. "Share this project" mints one
//! from a bound [`SyncEndpoint`](crate::SyncEndpoint); "accept a ticket" parses
//! one and dials it. There is no account, no server, no invite list and nothing
//! to sign up for, which is the point — a Wobu share is a string, and the string
//! is the entire mechanism.
//!
//! ## What is in one, and what each part is actually for
//!
//! - **The project ULID.** Which world this is about. It is the same id
//!   the opening exchange puts on the wire and the same one `project.json` carries,
//!   because a share that renamed the project between two machines would be two
//!   projects.
//! - **The peer's [`EndpointAddr`].** Its [`EndpointId`] — the ed25519 public key
//!   that *is* its TLS certificate — plus whatever transport paths it knew about
//!   when the ticket was minted. The paths are why a ticket works where a bare id
//!   does not: an id has to be looked up, and a lookup needs infrastructure and a
//!   round trip. A ticket carries the answer with it.
//! - **The [`Grant`].** Thirty-two random bytes. Read the next section before
//!   assuming anything about it, because it is the part most likely to be
//!   mistaken for something it is not.
//!
//! ## The grant is not a key, and must never become one
//!
//! [#77](https://github.com/krazyjakee/wobu/issues/77) calls this "a shared
//! secret", and the phrase invites exactly the mistake the crate documentation
//! forbids. A `wobu/sync/1` connection is already end-to-end encrypted and
//! mutually authenticated before this crate sees it: iroh is QUIC with TLS 1.3
//! and each endpoint's public key is its certificate. There is no confidentiality
//! left to add and no identity left to prove. A secret in a ticket used to
//! encrypt, to key, to salt, to MAC or to challenge would be application code
//! re-deriving — worse — a property the transport already has.
//!
//! So the grant is not a key. It is a **capability**: evidence that the holder
//! was *given* this ticket, rather than having read a ULID off a folder. That
//! distinction is the reason a ticket is worth protecting at all, and it is worth
//! spelling out why:
//!
//! Every other field in a ticket is already public. The project ULID travels
//! inside `project.json` to everyone who can mount the share. The endpoint id is
//! a peer's TLS certificate, exchanged in the clear on every connection attempt.
//! The transport addresses are where a machine answers the phone. Strip the grant
//! out and a "ticket" is a restatement of things anybody near the project already
//! has — which would make the issue's central rule (a ticket is a credential;
//! keep it out of `project.json`) a stylistic preference rather than a fact. The
//! grant is the only thing in here that is not otherwise obtainable, and
//! therefore the only thing that makes the sentence true.
//!
//! `wobu/sync/1` presents the grant on a second QUIC stream. It is deliberately
//! absent from the opening message: the ALPN *is* the version (see [`crate`]),
//! and adding a field to that existing message would be a silent wire change an
//! old peer reads as garbage. The acceptor hands the project and optional grant
//! to [`crate::Projects::admits`] and keeps only its bool. Consequently a wrong
//! grant and an unknown project produce the same `not_held` answer, QUIC close
//! code, fixed reason, and fieldless [`crate::Error::ProjectNotHeld`].
//!
//! [`crate::SyncEndpoint::connect_ticket`] presents `Some(grant)`. A bare
//! [`crate::SyncEndpoint::connect`] presents `None`, leaving the policy with the
//! application: a permissive test implementation may admit it, while Wobu's
//! sync manager requires the persisted project grant.
//!
//! ## Revocation does not exist
//!
//! There is no way to take a ticket back. No expiry, no serial number, no list to
//! remove it from. Anyone holding the string holds it until the whole project is
//! unshared, which invalidates every ticket at once. The UI has to say that out loud rather
//! than offering a "revoke" button that quietly does nothing; a control that
//! implies a power the system does not have is worse than no control.
//!
//! ## iroh's `Ticket` trait rather than our own base32
//!
//! [`iroh_tickets::Ticket`] gives the canonical form for free: a lowercase kind
//! prefix, then unpadded base32 of the bytes. Reusing it buys three things a
//! hand-rolled encoder would have to earn separately. A Wobu ticket looks like an
//! iroh ticket, because it is one, so a user who has seen a `blob…` ticket is not
//! learning a second format. The base32 alphabet, the case folding on the way in
//! and the "wrong prefix" error are somebody else's tested code rather than ours.
//! And the [`EndpointAddr`] inside is encoded by the same mirror-struct pattern
//! `iroh_tickets::endpoint::EndpointTicket` uses, so a ticket's address bytes and
//! iroh's own do not drift apart.
//!
//! What is *not* delegated is the shape of the payload: `Wire` is ours, it is an
//! enum with one variant today, and that enum is the version. A future `Wire::V2`
//! can be added without every `wobuproject…` string in existence becoming
//! unparseable, which is the only kind of versioning that matters for a format
//! whose instances live in other people's chat logs.
//!
//! The prefix is `wobuproject` rather than `wobu` on purpose. A kind prefix is a
//! namespace and there is only one of it: spend `wobu` on the first ticket type
//! and the second one has nowhere to go except a string that starts with `wobu`
//! too — and since base32's alphabet is a subset of ASCII letters, `wobuinvite…`
//! would strip the `wobu` prefix cleanly and then fail somewhere less obvious.
//!
//! ## `Display` hands out the credential; `Debug` does not
//!
//! [`Ticket`]'s [`Display`](fmt::Display) is the token, because producing the
//! token is the entire feature. Its [`Debug`](fmt::Debug) is hand-written and
//! prints the project, the peer's alias and how many routes it knows — the three
//! things anybody debugging a share wants — and not the grant. The same argument
//! `identity.rs` makes about an ed25519 secret applies with less force and in the
//! same direction: a `{:?}` in a log line should not be how a credential escapes,
//! and `#[derive(Debug)]` is how that happens.
//!
//! ## Where a ticket is kept, which is not here
//!
//! Beside the keychain entry, in local app data, and **never in `project.json`**
//! — a project folder is copied to a NAS, a USB stick and a git repo, so a
//! credential inside one is a credential handed to everyone who can mount it.
//!
//! This module deliberately does not write that file. `wobu-sync` is a transport
//! crate with no filesystem code in it, and `identity.rs` argues at length that
//! the moment a *file* can supply a secret here, the next question is which file
//! and somebody's answer is one inside the project. So [`Ticket`] is
//! [`Serialize`]/[`Deserialize`] as its own canonical string and the shell — which
//! already owns app-data paths and already keeps provider keys — persists it.
//! One serialised form rather than two, so a ticket in a config file and a ticket
//! in a chat message are the same characters.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use iroh::{EndpointAddr, EndpointId, TransportAddr};
use iroh_tickets::{ParseError, Ticket as IrohTicket};
use rand::Rng as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use subtle::ConstantTimeEq as _;
use wobu_core::Id;

use crate::error::Error;

/// The invitation half of a ticket: thirty-two bytes that came from nowhere but
/// the operating system's random number generator.
///
/// Not a key. See the module documentation — there is nothing to encrypt and
/// nobody to authenticate that TLS has not already dealt with, and the one thing
/// this is for is telling "I was invited" apart from "I read a ULID off a shared
/// folder". Nothing in this crate derives anything from it, and nothing may.
///
/// Thirty-two bytes because it is a bearer capability with no expiry and no way
/// to withdraw it, and a capability nobody can revoke is one nobody may guess.
/// `Copy`, because it is small and because the alternative is callers cloning a
/// ticket to read one field of it.
///
/// ## What checking it proves
///
/// [#90](https://github.com/krazyjakee/wobu/issues/90) made this an admission
/// input. It distinguishes a peer who was given a ticket from one who assembled
/// its public address and project fields elsewhere. It proves no identity — TLS
/// already did that — and supplies no confidentiality.
///
/// The comparison is constant-time through this type's [`PartialEq`]
/// implementation. That does not make the grant a cryptographic key; it keeps
/// an ordinary comparison from leaking a bearer capability byte by byte.
///
/// It still does not provide revocation. One grant is minted per project per
/// installation and copied into every ticket that installation hands out.
/// Removing the share stops all of those tickets; one collaborator cannot be
/// removed while another keeps the same grant.
#[derive(Clone, Copy, Eq, Serialize, Deserialize)]
pub struct Grant([u8; 32]);

impl PartialEq for Grant {
    fn eq(&self, other: &Grant) -> bool {
        bool::from(self.0.ct_eq(&other.0))
    }
}

impl Grant {
    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Grant {
        Grant(bytes)
    }

    /// A fresh grant from the OS random number generator.
    ///
    /// `rand::rng()`, which is seeded from the platform's entropy source. This is
    /// the only randomness in the crate and it is not a cryptographic operation:
    /// it asks the OS for bytes and stores them. There is no derivation, no
    /// stretching and no key schedule, which is what keeps it on the right side
    /// of the rule in [`crate`].
    pub fn generate() -> Grant {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        Grant(bytes)
    }

    /// The raw bytes used by the dedicated presentation stream.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Prints nothing. The whole value is the secret, so there is no useful prefix to
/// show and every character of it is one that should not be in a log file.
impl fmt::Debug for Grant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Grant(..)")
    }
}

/// A project share, as one pasteable string.
///
/// Cheap to clone — an endpoint id, a small set of addresses and thirty-two
/// bytes — and immutable once minted, because the string somebody already pasted
/// is the source of truth and a mutable ticket would be a second one.
#[derive(Clone, PartialEq, Eq)]
pub struct Ticket {
    project: Id,
    addr: EndpointAddr,
    grant: Grant,
}

impl Ticket {
    /// Mint a ticket for a project, at an address, with a grant.
    ///
    /// The grant is a parameter rather than something minted here, and that is
    /// the API saying something: a project shared twice should be shared with the
    /// *same* grant, or the ticket a collaborator pasted into their notes last
    /// month stops being the ticket this machine would honour. Persist the grant
    /// with the share (see the module documentation) and pass it back in;
    /// [`Grant::generate`] is for the first time only.
    ///
    /// [`crate::SyncEndpoint::ticket`] is the same thing with the address filled
    /// in from a bound endpoint, and is what the app should call.
    pub fn new(project: Id, addr: EndpointAddr, grant: Grant) -> Ticket {
        Ticket { project, addr, grant }
    }

    /// The project this share is about.
    pub fn project(&self) -> Id {
        self.project
    }

    /// The peer to dial: its public key, which is also its TLS certificate, so
    /// nothing has to check that the machine which answers is the one named here
    /// — iroh will not hand over a connection to anybody else.
    pub fn peer(&self) -> EndpointId {
        self.addr.id
    }

    /// The full address, including whatever transport paths the minting endpoint
    /// knew about at the time.
    ///
    /// Empty of routes if the ticket was minted before
    /// [`crate::SyncEndpoint::online`] resolved, in which case dialling it needs
    /// an address lookup service and works only where one exists.
    pub fn addr(&self) -> &EndpointAddr {
        &self.addr
    }

    /// The grant. See [`Grant`], and see the module documentation before using it
    /// for anything.
    pub fn grant(&self) -> Grant {
        self.grant
    }

    /// The peer's short name — `amber-heron-4f1a` — for showing beside the token.
    ///
    /// Display only, always. Twenty-eight bits is a name and not a key, and
    /// [`wobu_core::peer`] explains why nothing may ever decide anything with it.
    /// It is useful here for one thing: a person who has been sent a ticket out
    /// of band can be told which peer it names without reading sixty-four hex
    /// characters.
    pub fn alias(&self) -> String {
        wobu_core::peer::alias(self.peer().as_bytes())
    }

    /// Whether this ticket names a relay, which is the same question as "will
    /// this work from outside my LAN".
    ///
    /// A direct address is a socket on some network the minting machine was on;
    /// pasted to somebody elsewhere it is unroutable, and the dial fails with no
    /// clue as to why. A relay path always works, at the cost of a hop. This is
    /// what a "share" dialog should check before letting a user copy the string —
    /// and under [`crate::Reach::Internet`] the fix is to await
    /// [`crate::SyncEndpoint::online`] first.
    pub fn is_relayed(&self) -> bool {
        self.addr.addrs.iter().any(TransportAddr::is_relay)
    }

    /// Whether accepting this ticket means cloning the project or joining a
    /// replica already on this machine.
    ///
    /// One project ULID is one world, however many folders it is sitting in, so a
    /// second copy of a project already held would be two replicas on one machine
    /// syncing against each other — which is not a share, it is a bug with a
    /// progress bar. The check is against [`crate::Projects`] rather than against
    /// a path, because "already held" is a fact about the id and not about where
    /// somebody happened to put the folder.
    pub fn disposition(&self, projects: &dyn crate::Projects) -> Disposition {
        if projects.admits(&self.project, Some(&self.grant)) {
            Disposition::Join
        } else {
            Disposition::Clone
        }
    }
}

/// What accepting a ticket should do.
///
/// Two variants rather than a `bool`, because `if ticket.is_new()` at a call site
/// reads as a question about the ticket and this is a question about the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// This machine already holds the project. Start syncing the replica that is
    /// here; do not copy anything.
    Join,
    /// This machine has never seen the project. It has to be cloned into a
    /// directory the user picks before there is anything to sync.
    Clone,
}

/// The canonical string: `wobuproject` followed by unpadded lowercase base32.
///
/// This is the credential leaving the process, and it is the only impl that does
/// it — [`fmt::Debug`] below deliberately does not, so a token cannot reach a log
/// line by way of a struct that happened to `{:?}` one of these.
impl fmt::Display for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&IrohTicket::encode_string(self))
    }
}

/// The project, the peer's alias and the number of routes. Not the grant, and
/// not the token.
impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ticket")
            .field("project", &self.project.to_string())
            .field("peer", &self.alias())
            .field("routes", &self.addr.addrs.len())
            .finish_non_exhaustive()
    }
}

impl FromStr for Ticket {
    type Err = Error;

    /// Parses what a user pasted.
    ///
    /// Trims and lowercases first, because the realistic input is a line out of a
    /// chat client and a share that failed because somebody selected one
    /// character too many would be a support conversation rather than a security
    /// property.
    ///
    /// The lowercasing is not redundant with iroh's decoder, which folds the case
    /// of the base32 *body* but matches [`KIND`](IrohTicket::KIND) exactly — so
    /// `Wobuproject…`, out of a phone keyboard that capitalises the start of
    /// anything resembling a sentence, would otherwise fail on the prefix while
    /// the hard part of the string was fine. Folding the whole thing is safe
    /// because every character a ticket can contain is a lowercase ASCII letter
    /// or a digit; the assertion in `a_ticket_is_one_word_a_chat_client_will_not_mangle`
    /// is what keeps that true.
    fn from_str(s: &str) -> Result<Ticket, Error> {
        IrohTicket::decode_string(&s.trim().to_ascii_lowercase()).map_err(|_| Error::NotATicket)
    }
}

/// Always the canonical string, in every serde format.
///
/// iroh's own tickets have two serialised forms — the string when the format is
/// human-readable, the parts when it is not. This has one, and the reason is that
/// the string is what a person handles. A ticket in the shell's app-data file and
/// a ticket in a chat message being byte-identical means a user can copy one out
/// of the other, and a support conversation can ask them to.
impl Serialize for Ticket {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Ticket {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Ticket, D::Error> {
        let token = String::deserialize(deserializer)?;
        token.parse().map_err(serde::de::Error::custom)
    }
}

impl IrohTicket for Ticket {
    /// The kind prefix, and the namespace. See the module documentation for why
    /// it is not just `wobu`.
    const KIND: &'static str = "wobuproject";

    fn encode_bytes(&self) -> Vec<u8> {
        let wire = Wire::V1(V1 {
            project: self.project.to_bytes(),
            peer: self.addr.id,
            routes: self.addr.addrs.clone(),
            grant: self.grant,
        });
        postcard::to_stdvec(&wire).expect("a ticket is three fixed-size fields and a set of addrs")
    }

    fn decode_bytes(bytes: &[u8]) -> Result<Ticket, ParseError> {
        let Wire::V1(v1) = postcard::from_bytes(bytes)?;
        Ok(Ticket {
            project: Id::from_bytes(v1.project),
            addr: EndpointAddr { id: v1.peer, addrs: v1.routes },
            grant: v1.grant,
        })
    }
}

/// The ticket's bytes, versioned by being an enum.
///
/// One variant today. A second one can be added and old strings keep parsing,
/// which is the only property that matters for a format that lives in other
/// people's chat logs. Postcard encodes the discriminant as a varint, so the
/// version costs one byte.
#[derive(Serialize, Deserialize)]
enum Wire {
    V1(V1),
}

/// Version one, written out flat rather than by serialising the public types.
///
/// The ULID is sixteen bytes rather than [`Id`]'s own serde form, because that
/// form belongs to the `ulid` crate and could reasonably change without anything
/// in Wobu noticing until a ticket minted last year stopped parsing. The address
/// is split into an id and a set of [`TransportAddr`] exactly as
/// `iroh_tickets::endpoint::EndpointTicket` splits it, so the address bytes in a
/// Wobu ticket and in an iroh one are the same bytes and stay that way.
#[derive(Serialize, Deserialize)]
struct V1 {
    project: [u8; 16],
    peer: EndpointId,
    routes: BTreeSet<TransportAddr>,
    grant: Grant,
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr};

    use iroh::SecretKey;
    use wobu_core::new_id;

    use super::*;

    /// A ticket for a peer that answers on one loopback socket. Enough shape to
    /// exercise the encoding; `tests/tickets.rs` is where one gets dialled.
    fn ticket() -> Ticket {
        let peer = SecretKey::generate().public();
        let addr = EndpointAddr::from_parts(
            peer,
            [TransportAddr::Ip(SocketAddr::from((Ipv4Addr::LOCALHOST, 4433)))],
        );
        Ticket::new(new_id(), addr, Grant::generate())
    }

    /* ── the string ───────────────────────────────────────────────────────── */

    #[test]
    fn a_ticket_survives_being_pasted() {
        // The whole feature in one assertion: everything a share needs has to
        // come back out of the characters, because the characters are the only
        // thing that travels. A field that round-tripped as anything but itself
        // would be a share that connects to the wrong peer or syncs the wrong
        // world.
        let minted = ticket();

        let parsed: Ticket = minted.to_string().parse().unwrap();

        assert_eq!(parsed.project(), minted.project());
        assert_eq!(parsed.peer(), minted.peer());
        assert_eq!(parsed.addr(), minted.addr());
        assert_eq!(parsed.grant(), minted.grant());
        assert_eq!(parsed, minted);
    }

    #[test]
    fn a_ticket_is_one_word_a_chat_client_will_not_mangle() {
        // It gets pasted into Discord, Slack and an email client, all of which
        // linkify, wrap and autocorrect. Lowercase letters and digits only means
        // there is nothing in it for a client to turn into a link, a smart quote
        // or a line break — and no whitespace means selecting it is one
        // double-click.
        let token = ticket().to_string();

        assert!(token.starts_with("wobuproject"), "{token}");
        assert!(
            token.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "a ticket contains something other than lowercase base32: {token}"
        );
        assert!(!token.contains(char::is_whitespace), "{token}");
    }

    #[test]
    fn a_pasted_ticket_survives_the_whitespace_a_chat_client_adds() {
        // Selecting a line in a chat client takes the newline with it. A share
        // that failed on a trailing `\n` would be a support conversation, not a
        // security property.
        let minted = ticket();
        let sloppy = format!("  {minted}\n");

        assert_eq!(sloppy.parse::<Ticket>().unwrap(), minted);
    }

    #[test]
    fn a_ticket_that_has_been_through_an_autocapitalising_keyboard_still_reads() {
        // Base32 is case-insensitive and phones capitalise the first letter of
        // anything that looks like a sentence.
        let minted = ticket();

        assert_eq!(minted.to_string().to_uppercase().parse::<Ticket>().unwrap(), minted);
    }

    /* ── what is refused ──────────────────────────────────────────────────── */

    #[test]
    fn something_that_is_not_a_ticket_is_refused_rather_than_guessed_at() {
        // Every one of these is a real paste: the wrong clipboard entry, half a
        // token, a token with a character dropped by a line wrap. None of them
        // may produce a `Ticket`, because a ticket that parsed out of noise would
        // be a dial at an endpoint id nobody controls.
        let minted = ticket().to_string();
        for junk in [
            "",
            "hello",
            "https://example.com/share",
            &minted[..minted.len() / 2],
            &minted[1..],
            &format!("{minted}zzzz"),
            &minted.replace("wobuproject", "wobupro"),
        ] {
            assert!(junk.parse::<Ticket>().is_err(), "{junk:?} parsed as a ticket");
        }
    }

    #[test]
    fn another_kind_of_iroh_ticket_is_not_a_wobu_share() {
        // Wobu tickets and iroh's own are the same shape by design, which is
        // exactly why the prefix has to be load-bearing: an `endpoint…` ticket
        // decodes as valid base32 and would otherwise reach postcard.
        let peer = SecretKey::generate().public();
        let theirs = iroh_tickets::endpoint::EndpointTicket::new(EndpointAddr::from_parts(
            peer,
            [TransportAddr::Ip(SocketAddr::from((Ipv4Addr::LOCALHOST, 4433)))],
        ));

        assert!(theirs.to_string().parse::<Ticket>().is_err());
    }

    /* ── the credential does not leak ─────────────────────────────────────── */

    #[test]
    fn the_grant_does_not_survive_a_debug_dump() {
        // The classic leak, one level up from the one `identity.rs` guards: a
        // `{:?}` on the struct that happens to hold the credential. `Display` is
        // the deliberate way out and `Debug` must not be a second one.
        let ticket = ticket();

        let grant: String =
            ticket.grant().as_bytes().iter().map(|byte| format!("{byte:02x}")).collect();

        let dumped = format!("{ticket:?}");
        assert!(!dumped.contains(&ticket.to_string()), "the token survived Debug: {dumped}");
        assert!(!dumped.contains(&grant), "the grant survived Debug: {dumped}");
        // And a prefix of it, in case a future impl decides a few bytes are
        // harmless to print. They are not: this is a bearer capability with no
        // revocation, so a partial disclosure is a shortened search.
        assert!(!dumped.contains(&grant[..8]), "{dumped}");
        assert!(!format!("{:?}", ticket.grant()).contains(&grant[..8]));
        assert!(dumped.contains(&ticket.alias()), "the dump is still useful: {dumped}");
        assert!(dumped.contains(&ticket.project().to_string()), "{dumped}");
    }

    #[test]
    fn the_grant_is_the_only_part_of_a_ticket_that_is_not_already_public() {
        // The claim the whole "a ticket is a credential" rule rests on. The
        // project id is in `project.json`, the endpoint id is a TLS certificate
        // and the addresses are where a machine answers the phone — so two
        // tickets for the same project at the same address, differing only in the
        // grant, must be different strings. If they were not, a ticket would be
        // derivable from the folder and there would be nothing to protect.
        let peer = SecretKey::generate().public();
        let addr = EndpointAddr::from_parts(
            peer,
            [TransportAddr::Ip(SocketAddr::from((Ipv4Addr::LOCALHOST, 4433)))],
        );
        let project = new_id();

        let one = Ticket::new(project, addr.clone(), Grant::generate());
        let two = Ticket::new(project, addr, Grant::generate());

        assert_ne!(one.grant(), two.grant(), "two grants came out the same");
        assert_ne!(one.to_string(), two.to_string());
    }

    /* ── what a share dialog needs to know ────────────────────────────────── */

    #[test]
    fn a_ticket_with_no_relay_says_so() {
        // A direct address is a socket on the minting machine's LAN. Pasted to
        // somebody in another country it is unroutable and the dial fails with no
        // explanation, so a share dialog has to be able to ask this question
        // before it lets the string be copied.
        assert!(!ticket().is_relayed());

        let relayed = Ticket::new(
            new_id(),
            EndpointAddr::from_parts(SecretKey::generate().public(), [])
                .with_relay_url("https://relay.example/".parse().unwrap()),
            Grant::generate(),
        );
        assert!(relayed.is_relayed());
    }

    /* ── accepting ────────────────────────────────────────────────────────── */

    #[test]
    fn a_ticket_for_a_project_already_here_joins_rather_than_cloning() {
        // One ULID is one world however many folders it is in. Cloning a project
        // this machine already holds would leave two replicas syncing against
        // each other on one disk.
        struct Held(Id);
        impl crate::Projects for Held {
            fn admits(&self, project: &Id, _grant: Option<&Grant>) -> bool {
                *project == self.0
            }
        }

        let mine = ticket();
        let theirs = ticket();

        assert_eq!(mine.disposition(&Held(mine.project())), Disposition::Join);
        assert_eq!(theirs.disposition(&Held(mine.project())), Disposition::Clone);
    }

    /* ── the format is versioned ──────────────────────────────────────────── */

    #[test]
    fn the_wire_form_is_versioned_so_a_pasted_ticket_can_outlive_this_release() {
        // A ticket lives in somebody else's chat log. The first byte of the
        // payload being a variant tag is what lets a `V2` be added without every
        // string in existence becoming unparseable — and postcard costs one byte
        // for it.
        let bytes = IrohTicket::encode_bytes(&ticket());
        assert_eq!(bytes[0], 0, "the V1 discriminant is no longer the first byte");

        let mut future = bytes.clone();
        future[0] = 1;
        assert!(
            Ticket::decode_bytes(&future).is_err(),
            "an unknown ticket version decoded as V1 anyway"
        );
    }
}
