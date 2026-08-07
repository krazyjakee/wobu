use super::*;

/// A root that exists and then does not.
///
/// [`place`] asks the filesystem about the directories on the way, so these
/// cannot be tests against a made-up path — several of them plant a symlink
/// and one plants a directory where a file should be. Deref rather than a
/// getter so the call sites read as if it were the `&Path` it stands in for.
struct Root(PathBuf);

fn root() -> Root {
    let dir = std::env::temp_dir().join(format!("wobu-sync-place-{}", wobu_core::new_id()));
    fs::create_dir_all(&dir).unwrap();
    Root(dir)
}

impl std::ops::Deref for Root {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Root {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// #82 has to keep one of these in Tauri's managed state, which is
/// `Send + Sync + 'static` or it does not compile — and it would find that
/// out in the crate it cannot fix rather than in this one. A `Blobs` that
/// stopped being either is a change here, so the failure belongs here.
#[test]
fn the_handles_the_shell_has_to_hold_can_be_held() {
    fn shareable<T: Send + Sync + 'static>() {}
    shareable::<Blobs>();
    shareable::<crate::Config>();
    shareable::<Fetched>();
    shareable::<Offered>();
}

/* ── the paths that are allowed ───────────────────────────────────── */

#[test]
fn the_two_real_path_shapes_are_placeable() {
    let root = root();
    let hash = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

    for good in [
        format!("assets/originals/af/{hash}.png"),
        format!("assets/thumbs/af/{hash}.webp"),
        format!("assets/meshes/af/{hash}/model.glb"),
        "generations/2026-07/01ARZ3NDEKTSV4RRFFQ69G5FAV.json".to_string(),
    ] {
        let placed = place(&root, &good).unwrap_or_else(|e| panic!("{good} was refused: {e:?}"));
        assert!(placed.starts_with(&root));
        assert!(placed.to_string_lossy().ends_with(good.rsplit('/').next().unwrap()));
    }
}

/* ── the paths that are not ───────────────────────────────────────── */

#[test]
fn nothing_a_peer_can_write_reaches_outside_the_project_folder() {
    // The test that matters. Every entry here is a real technique rather
    // than a variation on one: prefix escapes, separator confusion, the
    // Unicode homoglyphs that a `..` filter written on characters would miss,
    // the Windows spellings that are a different file to the OS than to us,
    // and the device names that are not files at all.
    let root = root();

    for hostile in [
        // Traversal, plain and encoded.
        "../../../etc/passwd",
        "assets/../../etc/passwd",
        "assets/../../../../../../../../etc/shadow",
        "assets/originals/../../../.ssh/authorized_keys",
        "assets/./../../etc/passwd",
        "assets/x/../../../../root/.bashrc",
        // Absolute, POSIX and Windows and UNC and verbatim.
        "/etc/passwd",
        "/assets/originals/ab/x.png",
        "C:/Windows/System32/drivers/etc/hosts",
        "C:\\Windows\\win.ini",
        "\\\\server\\share\\x.png",
        "\\\\?\\C:\\Windows\\win.ini",
        "assets/C:/x.png",
        // Separator confusion: a name here, a separator there.
        "assets\\..\\..\\x.png",
        "assets/originals\\..\\..\\..\\x.png",
        "assets/originals/ab\\x.png",
        // Empty segments, which normalise differently on different hosts.
        "assets//../x.png",
        "assets///x.png",
        "//assets/x.png",
        // NTFS alternate data streams and drive-relative paths.
        "assets/originals/ab/x.png:Zone.Identifier",
        "assets/originals/ab/x.png::$DATA",
        // Trailing dots and spaces: Win32 strips them, so these are all one
        // file to Windows and four paths to a naive comparison.
        "assets/originals/ab/x.png.",
        "assets/originals/ab/x.png ",
        "assets/originals/ab/x.png...",
        "assets/originals/ab./x.png",
        // DOS device names, with and without an extension.
        "assets/originals/ab/NUL",
        "assets/originals/ab/nul.png",
        "assets/originals/ab/CON.png",
        "assets/originals/ab/com1.png",
        "assets/originals/LPT9/x.png",
        "assets/originals/ab/AUX.webp",
        // Unicode: fullwidth solidus, one-dot-leader pairs, the two-dot
        // leader, a right-to-left override, a zero-width space, and a
        // homoglyph `а` from Cyrillic. None of these is `..` or `/` to a
        // string comparison and all of them are refused for being non-ASCII,
        // which is why that rule is a whitelist and not a tidiness one.
        "assets/\u{ff0f}..\u{ff0f}etc/passwd",
        "assets/\u{2024}\u{2024}/x.png",
        "assets/\u{2025}/x.png",
        "assets/\u{202e}gnp.x/x.png",
        "assets/\u{200b}../x.png",
        "\u{430}ssets/originals/ab/x.png",
        "assets/caf\u{e9}.png",
        // Control characters, including the ones that truncate a C string or
        // end a line in a log.
        "assets/originals/ab/x\0.png",
        "assets/originals/ab/x\n.png",
        "assets/originals/ab/x\r\n.png",
        // Trees this crate does not carry. `nodes/` travels as a validated
        // payload with a validated slug; a blob path into it would be a
        // second way in with none of that.
        "nodes/character/kael-vantris.md",
        "project.json",
        ".wobu/index.sqlite",
        ".wobu/tmp/x.part",
        // Empty, and the bare prefixes.
        "",
        "assets",
        "assets/",
        "generations/",
    ] {
        let refused = place(&root, hostile);
        assert!(refused.is_err(), "{hostile:?} was placed at {:?}", refused.unwrap().display());
    }
}

#[test]
fn a_placed_path_is_under_the_root_even_when_the_root_has_a_dot_dot_in_it() {
    // `join` is lexical, so a root that is itself unnormalised would make the
    // containment check meaningless if it were done by string comparison
    // rather than by `strip_prefix` on components. `Blobs::open`
    // canonicalises, and this is what says the join does not depend on that
    // having happened.
    let root = root();
    let awkward = root.join("..").join(root.file_name().unwrap());

    let placed = join(&awkward, "assets/originals/ab/x.png").unwrap();

    assert!(placed.starts_with(&awkward));
    assert!(placed.ends_with("assets/originals/ab/x.png"));
}

#[test]
fn a_symlinked_directory_on_the_way_is_refused() {
    // The escape a lexical check cannot see, and the one a shared project
    // folder makes reachable: the string never leaves `assets/`, and the
    // filesystem does.
    let root = root();
    let elsewhere = root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();

    #[cfg(unix)]
    std::os::unix::fs::symlink(&elsewhere, root.join("assets/originals")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&elsewhere, root.join("assets/originals")).unwrap();

    assert!(join(&root, "assets/originals/ab/x.png").is_ok(), "lexically it is fine");
    assert_eq!(place(&root, "assets/originals/ab/x.png"), Err(Unplaceable::SymlinkedAncestor));
}

#[test]
fn a_symlink_at_the_target_is_not_quietly_replaced() {
    let root = root();
    let dir = root.join("assets/originals/ab");
    fs::create_dir_all(&dir).unwrap();
    fs::write(root.join("bait"), b"bait").unwrap();

    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("bait"), dir.join("x.png")).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(root.join("bait"), dir.join("x.png")).unwrap();

    assert_eq!(place(&root, "assets/originals/ab/x.png"), Err(Unplaceable::TargetIsNotAFile));
}

#[test]
fn a_directory_where_a_file_should_be_is_refused_rather_than_renamed_over() {
    let root = root();
    fs::create_dir_all(root.join("assets/originals/ab/x.png")).unwrap();

    assert_eq!(place(&root, "assets/originals/ab/x.png"), Err(Unplaceable::TargetIsNotAFile));
}

/* ── the reasons are the reasons ──────────────────────────────────── */

#[test]
fn each_rule_refuses_for_its_own_reason() {
    // A single `Refused` variant would let every one of these pass while
    // only the length check worked. Pinning the reason is what keeps the
    // rules honest about which of them is load bearing.
    let root = root();

    for (path, why) in [
        ("../etc/passwd", Unplaceable::NotSyncable),
        ("assets\\x.png", Unplaceable::NotSyncable),
        ("assets/x.png:s", Unplaceable::NotSyncable),
        ("assets/caf\u{e9}.png", Unplaceable::NotSyncable),
        ("assets/x.png.", Unplaceable::TrailingDotOrSpace),
        ("assets/x.png ", Unplaceable::TrailingDotOrSpace),
        ("assets/NUL", Unplaceable::ReservedDeviceName),
        ("assets/com4.png", Unplaceable::ReservedDeviceName),
    ] {
        assert_eq!(place(&root, path), Err(why), "{path:?}");
    }
}

#[test]
fn a_name_that_merely_begins_like_a_device_is_an_ordinary_file() {
    // The other half of the device-name rule. `COMMENT.png` starts with
    // `COM` and `CONTOUR.webp` starts with `CON`; a prefix match would refuse
    // both, and refusing a legitimate asset is a sync that silently never
    // converges.
    let root = root();

    for ordinary in ["assets/originals/ab/comment.png", "assets/thumbs/ab/CONTOUR.webp"] {
        assert!(place(&root, ordinary).is_ok(), "{ordinary} was refused");
    }
    assert!(!is_reserved_device_name("comment.png"));
    assert!(!is_reserved_device_name("com.png"));
    assert!(!is_reserved_device_name("coma.png"));
    assert!(is_reserved_device_name("COM1"));
    assert!(is_reserved_device_name("nul.tar.gz"));
}

/* ── the path and the hash have to mean the same file ─────────────── */

#[test]
fn an_original_whose_name_is_not_its_hash_is_refused() {
    // The empty-blob poisoning case, stated as the unit test the
    // integration test names. `nothing` is BLAKE3 of no input, which every
    // store can satisfy without asking a peer, so a pairing that names it
    // under somebody else's path is a zero-byte file at that path forever.
    let real = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    let other = "b3".repeat(32);

    assert!(agrees(&format!("assets/originals/af/{real}.png"), real));
    assert!(agrees(&format!("assets/originals/af/{real}.jpeg"), real));
    assert!(!agrees(&format!("assets/originals/b3/{other}.png"), real), "the poisoning case");
    // Right file, wrong shard: `<hh>` is the first two characters of the
    // hash and a mismatch is a file nothing in the workspace would write.
    assert!(!agrees(&format!("assets/originals/ff/{real}.png"), real));
    // Right hash, extra path segment, and no segments at all.
    assert!(!agrees(&format!("assets/originals/af/deep/{real}.png"), real));
    assert!(!agrees(&format!("assets/originals/{real}.png"), real));
    // No extension is still an original.
    assert!(agrees(&format!("assets/originals/af/{real}"), real));
}

#[test]
fn a_project_lora_path_and_wire_hash_must_name_the_same_content() {
    let real = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";
    let other = "b3".repeat(32);
    assert!(agrees(&format!("assets/loras/af/{real}.safetensors"), real));
    assert!(!agrees(&format!("assets/loras/af/{real}.safetensors"), &other));
    assert!(!agrees(&format!("assets/loras/ff/{real}.safetensors"), real));
    assert!(!agrees(&format!("assets/loras/af/{real}.bin"), real));
    assert!(!agrees(&format!("assets/loras/af/deep/{real}.safetensors"), real));
}

#[test]
fn project_loras_get_a_longer_per_file_relay_budget() {
    let hash = format!("af{}", "0".repeat(62));
    let path = format!("assets/loras/af/{hash}.safetensors");
    assert_eq!(timeout_for(&path, BLOB_TIMEOUT), LORA_BLOB_TIMEOUT,);
    assert_eq!(timeout_for("assets/originals/af/x.png", BLOB_TIMEOUT), BLOB_TIMEOUT);
    assert_eq!(timeout_for(&path, Duration::from_secs(4000)), Duration::from_secs(4000),);
    assert_eq!(timeout_for("assets/loras/af/x.safetensors", BLOB_TIMEOUT), BLOB_TIMEOUT);
}

#[test]
fn the_trees_with_no_derivable_name_are_left_alone() {
    // Thumbs are named after the *original's* hash and meshes after a
    // directory's, so neither can be checked this way, and generations are
    // ULIDs. A rule extended to them would refuse every legitimate entry.
    let real = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

    assert!(agrees("assets/thumbs/ab/abcd1234.webp", real));
    assert!(agrees("assets/meshes/ab/whatever/model.glb", real));
    assert!(agrees("generations/2026-07/01ARZ3NDEKTSV4RRFFQ69G5FAV.json", real));
}

/* ── the hash a peer wrote ────────────────────────────────────────── */

#[test]
fn only_one_spelling_of_a_digest_parses() {
    // `Hash::from_str` takes base32 as well as hex, and upper-cases before
    // decoding either. Both would let a peer name one blob two ways, which is
    // precisely what `is_content_hash` refuses one module over — so the
    // narrower check has to run first here or the exchange's rule would stop
    // being the process's rule.
    let hex = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262";

    assert!(parse_hash(hex).is_some());
    assert!(parse_hash(&hex.to_uppercase()).is_none(), "two spellings of one digest");
    assert!(parse_hash("").is_none());
    assert!(parse_hash(&hex[..63]).is_none());
    // Valid base32 for the same 32 bytes, which `Hash::from_str` would take.
    assert!(parse_hash("V4JUTOPV7GQ2NIBABTPKG3OMSSN4WJOJVXARFN6MTKJ4VZA7GJRA").is_none());
}
