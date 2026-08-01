//! The name a peer goes by, derived from the key that *is* that peer.
//!
//! A peer's identity is an ed25519 public key — iroh calls it an `EndpointId`
//! and prints it as sixty-four hex characters. That is the identity, and nothing
//! here weakens it. But sixty-four hex characters is not a thing to put in a
//! filename a person has to read, and `kael-vantris.conflict-a3f1…b207-…md` is a
//! name that tells a writer nothing except that something went wrong.
//!
//! So [`alias`] turns those thirty-two bytes into `amber-heron-4f1a`: two words
//! a person can say out loud, and four hex characters that are literally the
//! first four of the id as iroh prints it everywhere else.
//!
//! ## Derived, never assigned
//!
//! The obvious alternative is a nickname the user types, stored somewhere and
//! mapped to the key. It is rejected, and the reason is the whole point of #76:
//! **a mapping has to travel.** A nickname is only meaningful to a machine that
//! holds the table, so either the table goes into the project folder — onto the
//! share, into git, back to being a forgeable string anyone can edit — or every
//! collaborator sees a different name for the same person and
//! `kael-vantris.conflict-<peer>-<ts>.md` stops meaning the same thing on every
//! machine, which is exactly what it was made to mean.
//!
//! A derived alias has no table. Anyone holding the id can compute it, offline,
//! with this function, and they all get the same answer. That is worth more than
//! letting somebody call themselves Jake.
//!
//! ## What the alias is *not*
//!
//! It is not an authenticator and nothing may ever treat it as one. Twenty-eight
//! bits is a name, not a key: a peer who wanted to be mistaken for somebody else
//! could grind out a keypair whose alias collides in a few seconds of CPU. That
//! is fine for labelling a conflict file and fatal for deciding whether to hand
//! over a project, so the rule is that an alias is only ever *displayed*.
//! Anything making a decision compares the full [`EndpointId`], which is what
//! `wobu-sync` gets back from TLS rather than from anybody's claim.
//!
//! [`EndpointId`]: https://docs.rs/iroh

/// A peer alias, as a filename fragment: `amber-heron-4f1a`.
///
/// Always a valid slug in the sense [`crate::is_valid_slug`] means — lowercase
/// ASCII, digits and single hyphens — because it goes straight into a filename
/// on a share that may be read from Windows, and because `wobu-store`'s conflict
/// parser splits those filenames on hyphens.
///
/// The two words come from bytes the suffix does not cover, so all three parts
/// carry different bits of the key.
pub fn alias(id: &[u8; 32]) -> String {
    // The words are chosen with `%` over a power-of-two list, which is only
    // unbiased because the lists are exactly 64 long. The test below is what
    // keeps that true when somebody adds a word they liked.
    let adjective = ADJECTIVES[usize::from(id[2]) % ADJECTIVES.len()];
    let noun = NOUNS[usize::from(id[3]) % NOUNS.len()];
    format!("{adjective}-{noun}-{}{}", hex(id[0]), hex(id[1]))
}

/// How many characters of the printed id the alias ends with.
///
/// Four rather than eight because the suffix is there to be *checked*, not to be
/// unique on its own — a user comparing an alias against an id in a ticket reads
/// four characters and stops. Uniqueness is the id's job.
pub const SUFFIX_CHARS: usize = 4;

/// Lowercase hex, matching how iroh's `Display` prints a public key, so the
/// alias suffix is a genuine prefix of the id rather than a second encoding of
/// it that happens to look similar.
fn hex(byte: u8) -> String {
    format!("{byte:02x}")
}

/// Sixty-four adjectives. The count is load-bearing — see [`alias`].
///
/// Colours, light and texture. Deliberately bland: every one of these has to
/// read acceptably in front of every noun below, because the pairing is chosen
/// by a public key and nobody gets to veto their own name.
const ADJECTIVES: &[&str] = &[
    "amber", "ashen", "azure", "blue", "bright", "bronze", "calm", "clear", "cobalt", "copper",
    "coral", "crimson", "dusky", "eager", "early", "emerald", "fair", "first", "gentle", "gilded",
    "glad", "golden", "grey", "hidden", "high", "humble", "indigo", "ivory", "jade", "keen",
    "late", "lilac", "little", "lone", "lucid", "mellow", "mild", "misty", "mossy", "muted",
    "narrow", "noble", "olive", "opal", "patient", "pearl", "quiet", "rapid", "ruby", "russet",
    "sable", "saffron", "scarlet", "silent", "silver", "slate", "still", "sunlit", "teal", "tidal",
    "umber", "velvet", "wild", "woven",
];

/// Sixty-four nouns. The count is load-bearing — see [`alias`].
///
/// Landscape and birds, for the same reason as above and because a worldbuilding
/// tool naming its users after network hardware would be a small betrayal.
const NOUNS: &[&str] = &[
    "alder", "anchor", "arbour", "arrow", "aspen", "basin", "beacon", "bell", "birch", "bridge",
    "brook", "cairn", "cedar", "cliff", "comet", "cove", "crane", "delta", "ember", "falcon",
    "fern", "ferry", "forge", "fox", "garden", "gate", "glade", "harbour", "hawk", "heron", "ibis",
    "island", "kestrel", "lantern", "ledge", "linnet", "maple", "marsh", "meadow", "moth",
    "orchard", "otter", "owl", "pine", "plover", "quarry", "quill", "raven", "reef", "ridge",
    "rook", "rowan", "sail", "sparrow", "spire", "spring", "stone", "swift", "thicket", "thorn",
    "tide", "vale", "willow", "wren",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A key whose first four bytes are known, so the shape of the output can be
    /// asserted rather than merely observed.
    fn key(first: [u8; 4]) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        bytes[..4].copy_from_slice(&first);
        bytes
    }

    #[test]
    fn the_lists_are_exactly_sixty_four_long() {
        // Not tidiness. `alias` indexes with `%`, and a list whose length does
        // not divide 256 makes the first `256 % len` entries more likely than
        // the rest — a bias in the one property this function exists to have.
        assert_eq!(ADJECTIVES.len(), 64, "the adjective list is no longer a power of two");
        assert_eq!(NOUNS.len(), 64, "the noun list is no longer a power of two");
    }

    #[test]
    fn the_same_key_always_produces_the_same_name() {
        // The claim the whole design rests on: there is no table, so two
        // machines agree only if this is a pure function of the id. A conflict
        // file named on one laptop has to be read as the same person's on
        // another.
        let id = key([0x4f, 0x1a, 0x00, 0x1d]);
        assert_eq!(alias(&id), alias(&id));
        assert_eq!(alias(&id), "amber-heron-4f1a");
    }

    #[test]
    fn the_suffix_is_the_start_of_the_id_as_iroh_prints_it() {
        // The point of the suffix. A user comparing an alias in a filename
        // against an id in a ticket should be reading the same characters, not
        // a second encoding that happens to look similar. `wobu-sync` has the
        // other half of this test, against a real `EndpointId`.
        let id = key([0xa3, 0x07, 0x02, 0x03]);
        let name = alias(&id);
        assert!(name.ends_with("a307"), "{name}");
        assert_eq!(name.rsplit('-').next().unwrap().len(), SUFFIX_CHARS);
    }

    #[test]
    fn the_words_come_from_bytes_the_suffix_does_not() {
        // Otherwise the words would be decoration: two aliases differing only in
        // the suffix would be as informative as the suffix alone, and the
        // memorable half of the name would carry no bits at all.
        let a = alias(&key([0x00, 0x00, 0x01, 0x02]));
        let b = alias(&key([0x00, 0x00, 0x03, 0x04]));
        assert_ne!(a, b, "the words ignored bytes 2 and 3");
        assert!(a.ends_with("0000") && b.ends_with("0000"));
    }

    #[test]
    fn an_alias_is_a_filename_a_share_will_accept() {
        // It is written into a `.conflict-` sibling on an SMB share that may be
        // read from Windows, and `wobu-store`'s parser splits those names on
        // hyphens. Every byte value is tried because the id is a public key and
        // there is no such thing as an unusual one.
        for a in 0u8..=255 {
            for b in 0u8..=255 {
                let name = alias(&key([0xff, 0x00, a, b]));
                assert!(crate::is_valid_slug(&name), "{name} is not a usable filename");
            }
        }
    }

    #[test]
    fn an_alias_can_never_be_mistaken_for_the_timestamp_beside_it() {
        // `wobu-store`'s conflict parser anchors on `%Y%m%dT%H%M%SZ`, which
        // needs a literal uppercase `T` and `Z`. An alias is lowercase
        // throughout, so the split can never land inside the peer's name — the
        // failure that would silently mislabel every conflict card.
        for byte in 0u8..=255 {
            let name = alias(&key([byte, byte, byte, byte]));
            assert!(!name.contains('T') && !name.contains('Z'), "{name}");
        }
    }
}
