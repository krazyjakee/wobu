//! Which peer this installation is, as far as a filename is concerned.
//!
//! Until #76 this was `$USER`, falling back to the literal string `"user"`. Both
//! halves were wrong in the same way. Two collaborators on default installs were
//! the same person, so `kael-vantris.conflict-user-20260731T142211Z.md` told a
//! writer nothing at all; and a name that comes out of an environment variable
//! is a name anybody can put on somebody else's work.
//!
//! What replaces it is [`wobu_core::peer::alias`] over an iroh `EndpointId` — an
//! ed25519 public key whose secret half is in the OS keychain, per installation,
//! never in the project folder. `wobu-sync` owns that key and this crate never
//! sees it; what arrives here is the alias, which is a short public name derived
//! from the public half and carries nothing secret.
//!
//! Not to be confused with [`crate::Peer`], which is somebody *else* — a
//! collaborator in the presence list. This module is only ever about us.
//!
//! ## Why this is a process-wide slot and not an argument
//!
//! A peer identity belongs to the *installation*, not to a project — that is the
//! whole reason the key is in the keychain rather than in the folder — so a
//! `Project::open(path, who_i_am)` would be threading a constant through every
//! call site and every test in order to say the same thing each time. Worse, it
//! would be a parameter, and a parameter is a thing a caller can get wrong: two
//! projects open in one window under two different names is not a state anything
//! downstream is prepared for, since [`crate::Conflict::mine`] compares against
//! it.
//!
//! So it is set once, by the app, before any project opens, and read from
//! everywhere else. [`install`] refuses a second value rather than replacing the
//! first, because a conflict sibling named under one alias and a later one named
//! under another, in the same session, is a folder nobody can read back.
//!
//! ## The unattributed fallback
//!
//! [`alias`] has no `Result` and always answers. If nothing was installed —
//! `wobu-sync` is not wired into the shell yet, or a test opened a project
//! directly — it mints one from a fresh ULID, once, for the life of the process.
//!
//! That is a genuine degradation and it is worth naming: the alias is stable
//! within a run and different on the next one, so a person's conflict files stop
//! accumulating under one name. It is chosen over `"unknown"` deliberately,
//! because the specific failure #76 exists to fix is *two people sharing a
//! name* — a folder where every sibling says `unknown` is the `$USER` bug again
//! with the serial numbers filed off, and the version of it that is impossible
//! to notice.

use std::sync::OnceLock;

/// Set once, read for the rest of the process.
///
/// `OnceLock` rather than a `RwLock`: the value is a fact about the machine, not
/// a setting, and the type is what makes "it cannot change under a reader" true
/// rather than merely intended.
static ALIAS: OnceLock<String> = OnceLock::new();

/// Tell this process who it is. Returns whether the value was taken.
///
/// `false` means either that an alias was already installed or that this one was
/// empty, and in both cases the caller's argument is *not* in effect. Losing
/// this race is not an error — the app calls it once at startup — but it is
/// worth reporting rather than swallowing, because the second caller is either a
/// bug or the app being started twice inside one process.
pub fn install(alias: &str) -> bool {
    let alias = alias.trim();
    // An empty alias would produce `kael-vantris.conflict--20260731T142211Z.md`,
    // which `conflict::parse` reads as an unlabelled sibling — a card that says
    // "someone, at some point" when we in fact knew who. Refusing is better than
    // storing a name that unnames itself.
    if alias.is_empty() {
        return false;
    }
    ALIAS.set(alias.to_owned()).is_ok()
}

/// The name this installation stamps on a conflict sibling, and the name a
/// collaborator sees in the presence list.
///
/// Shared between the two so they cannot disagree about who a person is: a
/// presence entry the user cannot match to the conflict file beside it is worse
/// than either on its own.
pub fn alias() -> &'static str {
    ALIAS.get_or_init(unattributed)
}

/// A name for a process that was never told who it is.
///
/// Derived through the same function a real peer's alias comes from, so it is
/// the same shape — a slug the conflict parser reads back, indistinguishable in
/// a filename from an attributed one. That is on purpose: the sibling is
/// somebody's only copy of a paragraph either way, and a name that announced
/// itself as second-class would invite something downstream to treat the file as
/// second-class too.
///
/// The ULID is hashed rather than used directly because its leading bytes are a
/// millisecond timestamp, and [`wobu_core::peer::alias`] takes its suffix from
/// the leading bytes — so two installs started in the same second would be
/// handed near-identical names by the one part of the alias meant to tell them
/// apart.
fn unattributed() -> String {
    let seed = blake3::hash(wobu_core::new_id().to_string().as_bytes());
    wobu_core::peer::alias(seed.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alias_is_a_filename_the_conflict_parser_reads_back() {
        // The whole contract this module has with `atomic::guarded_write` and
        // `conflict::parse`. `slugify` is applied on the way into a filename, so
        // an alias that was not already a slug would be silently reshaped and
        // stop matching the one in the presence list.
        let alias = unattributed();
        assert!(wobu_core::is_valid_slug(&alias), "{alias} is not a usable filename");
        assert_eq!(wobu_core::slugify(&alias).unwrap(), alias, "{alias} was reshaped");

        let name = format!("kael-vantris.conflict-{alias}-20260731T142211Z.md");
        let parsed = crate::conflict::parse(&name).expect("a sibling");
        assert_eq!(parsed.peer.as_deref(), Some(alias.as_str()));
        assert_eq!(parsed.stem, "kael-vantris");
    }

    #[test]
    fn two_processes_that_were_never_told_who_they_are_are_still_different_people() {
        // The `$USER` bug in its purest form: everybody called `user`, every
        // conflict file unattributable, and no way to tell from the folder that
        // anything is wrong. A fallback that collides is worse than no fallback.
        let names: std::collections::HashSet<String> = (0..64).map(|_| unattributed()).collect();
        assert_eq!(names.len(), 64, "the fallback repeats itself");
    }

    #[test]
    fn the_alias_does_not_change_underneath_a_reader() {
        // Two siblings written in one session under two different names is a
        // folder that cannot be read back, so the slot latches. `install`
        // may or may not have run by the time this test does — the tests in
        // this crate share a process — which is exactly why the assertion is
        // about stability rather than about a particular value.
        let first = alias();
        assert!(!first.is_empty());
        assert_eq!(first, alias());
        assert!(!install(""), "an empty alias must never be installed");
        assert_eq!(first, alias(), "installing changed an alias already in use");
    }
}
