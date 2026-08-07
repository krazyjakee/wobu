use super::*;

/// Three hashes that are definitely different from each other. Real BLAKE3
/// hex, because `decide` compares strings and a test using `"a"`/`"b"` would
/// pass just as well against a comparison that had been broken into
/// something length-dependent.
fn h(bytes: &[u8]) -> String {
    atomic::hash_bytes(bytes)
}

/* ── the truth table, row by row ──────────────────────────────────── */

#[test]
fn we_are_at_the_base_and_they_moved_so_we_take_theirs() {
    let (base, theirs) = (h(b"v1"), h(b"v2"));
    assert_eq!(decide(Some(&base), Some(&theirs), Some(&base)), Decision::FastForward);
}

#[test]
fn we_moved_and_they_are_at_the_base_so_they_need_ours() {
    let (base, ours) = (h(b"v1"), h(b"v2"));
    assert_eq!(decide(Some(&ours), Some(&base), Some(&base)), Decision::SendOurs);
}

#[test]
fn both_moved_to_the_same_bytes_and_nobody_has_to_choose() {
    // Two people typing the same paragraph, or one of them applying a
    // version they already had from a third machine. Nothing is at risk, so
    // a conflict card here would be pure noise — and noise is expensive,
    // because it teaches people to dismiss the card that one day matters.
    let (base, agreed) = (h(b"v1"), h(b"v2"));
    assert_eq!(decide(Some(&agreed), Some(&agreed), Some(&base)), Decision::Converged);
}

#[test]
fn both_moved_differently_and_that_is_the_only_conflict() {
    let (base, ours, theirs) = (h(b"v1"), h(b"mine"), h(b"yours"));
    assert_eq!(decide(Some(&ours), Some(&theirs), Some(&base)), Decision::Conflict);
}

#[test]
fn all_three_agreeing_is_not_work() {
    let same = h(b"v1");
    assert_eq!(decide(Some(&same), Some(&same), Some(&same)), Decision::InStep);
}

/* ── the invariant ───────────────────────────────────────────────── */

#[test]
fn no_base_is_concurrent_and_never_a_fast_forward() {
    // The single most important assertion in this crate. `None` is the
    // ordinary state for a peer we have never synced this node with, and
    // reading it as "same" would fast-forward their file over ours on first
    // contact — silently, before anybody had a chance to look. The cost of
    // being wrong the other way is a conflict card nobody needed.
    let (ours, theirs) = (h(b"mine"), h(b"yours"));
    assert_eq!(decide(Some(&ours), Some(&theirs), None), Decision::Conflict);
    // And in the other direction, so this cannot pass by always saying
    // conflict when the base is missing.
    assert_ne!(decide(Some(&ours), Some(&theirs), None), Decision::FastForward);
    assert_ne!(decide(Some(&ours), Some(&theirs), None), Decision::SendOurs);
}

#[test]
fn no_base_with_identical_bytes_still_only_moves_the_base() {
    // The other half: never-synced must not mean never-agreeing. Two
    // machines holding the same file have nothing to transfer and nothing to
    // resolve, and the very first exchange between two peers with a shared
    // history is almost entirely this case.
    let same = h(b"identical");
    assert_eq!(decide(Some(&same), Some(&same), None), Decision::Converged);
}

#[test]
fn a_base_that_matches_neither_side_is_a_conflict_not_a_guess() {
    // Both machines moved on from an agreement neither still holds. There is
    // no third version to prefer, so there is nothing to do but ask.
    let (base, ours, theirs) = (h(b"old"), h(b"a"), h(b"b"));
    assert_eq!(decide(Some(&ours), Some(&theirs), Some(&base)), Decision::Conflict);
}

/* ── nodes one side does not have ────────────────────────────────── */

#[test]
fn a_node_only_they_have_and_never_agreed_on_is_theirs_to_give() {
    // How a node created on another machine arrives. Safe because there is
    // no local file — `apply` reads the destination and turns this into a
    // conflict the moment there is something there to lose.
    let theirs = h(b"new");
    assert_eq!(decide(None, Some(&theirs), None), Decision::FastForward);
}

#[test]
fn a_node_only_we_have_and_never_agreed_on_is_ours_to_send() {
    let ours = h(b"new");
    assert_eq!(decide(Some(&ours), None, None), Decision::SendOurs);
}

#[test]
fn a_node_that_went_missing_after_an_agreement_is_left_completely_alone() {
    // A deletion, on whichever side. M3 has no tombstones, so the two
    // available guesses are "resurrect it" and "delete ours" — and the
    // second one, driven by an absence, turns a half-mounted share into a
    // world-wide erase. Neither is guessed.
    let agreed = h(b"v1");
    assert_eq!(decide(None, Some(&agreed), Some(&agreed)), Decision::Deleted);
    assert_eq!(decide(Some(&agreed), None, Some(&agreed)), Decision::Deleted);
    // Including when the surviving side also edited it, which is the case a
    // "the other side deleted it, so delete ours" rule loses outright.
    let edited = h(b"v2");
    assert_eq!(decide(Some(&edited), None, Some(&agreed)), Decision::Deleted);
}

#[test]
fn a_node_neither_side_has_is_nothing_at_all() {
    assert_eq!(decide(None, None, None), Decision::InStep);
    assert_eq!(decide(None, None, Some(&h(b"v1"))), Decision::InStep);
}

#[test]
fn decide_never_writes_and_never_fails() {
    // Restating the shape as an assertion, because it is the reason every
    // row above can be tested without a filesystem: the decision is a total
    // function of three optional strings. Anything that made it return a
    // `Result`, or take a path, would move the most important logic in M3
    // somewhere it can only be tested through IO.
    for local in [None, Some("a"), Some("b")] {
        for remote in [None, Some("a"), Some("b")] {
            for base in [None, Some("a"), Some("b")] {
                let _: Decision = decide(local, remote, base);
            }
        }
    }
}

/* ── refusals ────────────────────────────────────────────────────── */

fn refusals(hashes: &[&str]) -> HashSet<String> {
    hashes.iter().map(|h| h.to_string()).collect()
}

#[test]
fn a_refused_version_arriving_again_is_not_a_conflict() {
    // The bug (#89) in one line. The user said no to these exact bytes from
    // this exact peer; a base cannot record that without lying about
    // agreement, so the refusal is its own fact and this is where it lands.
    let theirs = h(b"nadia's second act");
    let refused = refusals(&[&theirs]);
    assert!(already_refused(Decision::Conflict, Some(&theirs), Some(&refused)));
}

#[test]
fn refusing_one_version_does_not_suppress_a_later_different_one() {
    // **The trap this whole feature is made of, and the reason the hash is
    // in the primary key.** Rejecting Tuesday's paragraph must not silence
    // Wednesday's: a `(peer, node)` table would make the peer's later work
    // vanish on this machine with no card, no file and no trace, which is a
    // genuinely lost edit rather than a redundant card.
    let tuesday = h(b"nadia's second act");
    let wednesday = h(b"nadia's second act, rewritten");
    let refused = refusals(&[&tuesday]);

    assert!(already_refused(Decision::Conflict, Some(&tuesday), Some(&refused)));
    assert!(
        !already_refused(Decision::Conflict, Some(&wednesday), Some(&refused)),
        "a later version from the same peer was silently swallowed",
    );

    // And refusing the second one as well leaves the first still refused —
    // a node can be argued about more than once, so the set has to grow
    // rather than be replaced.
    let both = refusals(&[&tuesday, &wednesday]);
    assert!(already_refused(Decision::Conflict, Some(&tuesday), Some(&both)));
    assert!(already_refused(Decision::Conflict, Some(&wednesday), Some(&both)));
}

#[test]
fn a_refusal_can_only_ever_suppress_a_conflict() {
    // The safety bound, stated as an exhaustive assertion rather than as a
    // comment. Every other row of the table either writes to a node file or
    // moves a base, so a row in a database that could reach one would be a
    // row authorising an overwrite — and a corrupt, forged or simply
    // mistaken row must never be able to do that.
    let theirs = h(b"theirs");
    let refused = refusals(&[&theirs]);
    for decision in [
        Decision::InStep,
        Decision::FastForward,
        Decision::SendOurs,
        Decision::Converged,
        Decision::Deleted,
    ] {
        assert!(
            !already_refused(decision, Some(&theirs), Some(&refused)),
            "{decision:?} was suppressed by a refusal",
        );
    }
    assert!(already_refused(Decision::Conflict, Some(&theirs), Some(&refused)));
}

#[test]
fn nothing_recorded_for_this_node_refuses_nothing() {
    // The ordinary case — almost every node, on almost every sync — and the
    // one where an empty-set-means-everything bug would be catastrophic and
    // completely silent.
    let theirs = h(b"theirs");
    assert!(!already_refused(Decision::Conflict, Some(&theirs), None));
    assert!(!already_refused(Decision::Conflict, Some(&theirs), Some(&refusals(&[]))));
    // A node the peer does not have cannot match a refusal about bytes.
    assert!(!already_refused(Decision::Conflict, None, Some(&refusals(&[&theirs]))));
}

#[test]
fn the_predicate_is_pure_and_never_fails() {
    // The same statement `decide_never_writes_and_never_fails` makes, for
    // the same reason: the consult deliberately did not go inside `decide`,
    // and it earns that only by being as testable as `decide` is. A `&Path`
    // or a `Result` here would put the last decision in M3 behind IO.
    for decision in [Decision::InStep, Decision::Conflict, Decision::FastForward] {
        for remote in [None, Some("a"), Some("b")] {
            for set in [None, Some(refusals(&[])), Some(refusals(&["a"]))] {
                let _: bool = already_refused(decision, remote, set.as_ref());
            }
        }
    }
}

/* ── naming the sender ───────────────────────────────────────────── */

#[test]
fn a_peers_alias_is_the_one_its_own_machine_would_write() {
    // There is no table mapping ids to names — that is #76's whole argument
    // — so a sibling named here has to carry the same name the sender's own
    // `guarded_write` would have put on it. That is only true if this is the
    // same pure function over the same bytes.
    let mut id = [0u8; 32];
    id[..4].copy_from_slice(&[0x4f, 0x1a, 0x00, 0x1d]);
    let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(alias_of(&hex), wobu_core::peer::alias(&id));
    assert_eq!(alias_of(&hex), "amber-heron-4f1a");
}

#[test]
fn an_alias_is_always_a_filename_the_conflict_parser_reads_back() {
    // The contract with `atomic::conflict_sibling` and `conflict::parse`. A
    // peer id is a public key and there is no such thing as an unusual one,
    // so every alias this can produce has to be a slug — including the ones
    // from the fallback, which is reached by a hex string that is not one.
    for peer_id in [
        "0a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f9",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "not hex at all",
        "",
        // Right length, wrong alphabet.
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
    ] {
        let alias = alias_of(peer_id);
        assert!(wobu_core::is_valid_slug(&alias), "{peer_id} produced {alias}");
        let name = format!("kael-vantris.conflict-{alias}-20260731T142211Z.md");
        let parsed = conflict::parse(&name).expect("a sibling");
        assert_eq!(parsed.peer.as_deref(), Some(alias.as_str()));
        assert_eq!(parsed.target_file_name(), "kael-vantris.md");
    }
}

#[test]
fn an_unparseable_id_still_names_the_same_peer_every_time() {
    // A fallback that wandered would scatter one machine's siblings across
    // several names, which is the `$USER` bug inverted and just as
    // unreadable.
    assert_eq!(alias_of("not hex at all"), alias_of("not hex at all"));
    assert_ne!(alias_of("not hex at all"), alias_of("also not hex"));
}

#[test]
fn hex_decoding_is_exact_about_length_and_alphabet() {
    assert_eq!(endpoint_bytes(&"00".repeat(32)), Some([0u8; 32]));
    assert_eq!(endpoint_bytes(&"ff".repeat(32)), Some([0xffu8; 32]));
    assert_eq!(endpoint_bytes(&"0".repeat(63)), None, "63 characters is not an id");
    assert_eq!(endpoint_bytes(&"0".repeat(65)), None, "65 characters is not an id");
    assert_eq!(endpoint_bytes(&"g".repeat(64)), None, "g is not hex");
    // A multi-byte character can make `len()` 64 while the string is 63
    // characters, and indexing bytes then splits it in half.
    let sneaky = format!("é{}", "0".repeat(62));
    assert_eq!(sneaky.len(), 64);
    assert_eq!(endpoint_bytes(&sneaky), None, "a non-ASCII id must not be decoded");
}

/* ── placing a file ──────────────────────────────────────────────── */

#[test]
fn a_stem_is_taken_from_the_path_we_are_going_to_use() {
    assert_eq!(stem_of("nodes/character/kael-vantris.md"), "kael-vantris");
    assert_eq!(stem_of("kael-vantris.md"), "kael-vantris");
    assert_eq!(stem_of("nodes/character/kael"), "kael");
}
