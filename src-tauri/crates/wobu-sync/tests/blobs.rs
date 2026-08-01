//! Blobs over a real connection: two endpoints, two blob stores, two project
//! folders, one process, one loopback interface.
//!
//! Everything here moves real bytes through real QUIC and real BLAKE3 verified
//! streaming, and lands them in a real directory. That is not thoroughness for
//! its own sake — [#81] is the first thing in this crate that **writes to a
//! filesystem at a path a peer chose**, and none of the three failures that
//! matters is visible to a test that stops at the API.
//!
//! - A path that escapes the project folder is a test that has to look at the
//!   filesystem afterwards, because the function under test returns a plausible
//!   `Ok` either way.
//! - A transfer that leaves half a file behind is a test that has to interrupt a
//!   transfer, because the tidy path always tidies.
//! - A provider that serves the wrong bytes is a test that has to *make* one,
//!   because nothing this build sends is ever wrong.
//!
//! `no_path_a_peer_can_write_escapes_the_project_folder` is the one to read
//! first. It is the reason this file exists.
//!
//! What a green run here does not cover is the list `loopback.rs` and
//! `manifest.rs` both give: NAT traversal, holepunching, relay selection, and a
//! link slow enough for a timeout to be interesting. `Reach::Loopback` has no
//! relay and no address lookup, so nothing here can quietly acquire a dependency
//! on n0's infrastructure.
//!
//! [#81]: https://github.com/krazyjakee/wobu/issues/81

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;
use wobu_core::{Id, new_id};
use wobu_sync::blobs::Unplaceable;
use wobu_sync::{Blob, Blobs, Config, Projects, Session, Sessions, SyncEndpoint};

/* ── the rig ──────────────────────────────────────────────────────────────── */

struct Held(Id);

impl Projects for Held {
    fn holds(&self, project: &Id) -> bool {
        *project == self.0
    }
}

struct Sink(mpsc::UnboundedSender<Session>);

#[async_trait]
impl Sessions for Sink {
    async fn opened(&self, session: Session) {
        let _ = self.0.send(session);
    }
}

/// A directory that deletes itself.
///
/// Hand-rolled rather than `tempfile`, because a dev-dependency added to this
/// crate is a dependency somebody will later reach for in `src/` — and the two
/// things this needs are `create_dir_all` and a `Drop`.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        let dir = std::env::temp_dir().join(format!("wobu-sync-blobs-{}", new_id()));
        std::fs::create_dir_all(dir.join("project")).unwrap();
        std::fs::create_dir_all(dir.join("cache")).unwrap();
        Scratch(dir)
    }

    fn root(&self) -> PathBuf {
        self.0.join("project")
    }

    fn cache(&self) -> PathBuf {
        self.0.join("cache")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One machine: a project folder, a blob store outside it, and an endpoint
/// speaking both ALPNs.
///
/// Field order is drop order and it matters: the endpoint holds a clone of the
/// store, the store's actor runs on threads of its own, and the directory those
/// threads write into has to outlive both. Listing `scratch` first would delete
/// the store's database while it was still being written to, which is a race
/// that leaves empty directories behind in `/tmp` and, one day, a confusing
/// failure.
struct Machine {
    endpoint: SyncEndpoint,
    blobs: Blobs,
    inbox: mpsc::UnboundedReceiver<Session>,
    scratch: Scratch,
}

impl Machine {
    async fn new(project: Id) -> Machine {
        let scratch = Scratch::new();
        let blobs = Blobs::open(scratch.root(), scratch.cache())
            .await
            .expect("a fresh directory pair opens");
        let (sessions, inbox) = mpsc::unbounded_channel();
        let config = Config {
            open_timeout: Duration::from_millis(500),
            blobs: Some(blobs.clone()),
            ..Config::loopback()
        };
        let endpoint = SyncEndpoint::bind(config, Arc::new(Held(project)), Arc::new(Sink(sessions)))
            .await
            .expect("a loopback endpoint binds without a network");
        Machine { endpoint, blobs, inbox, scratch }
    }

    fn root(&self) -> PathBuf {
        self.scratch.root()
    }

    /// Write a file into this machine's project folder, hash it, and make it
    /// servable — which is what a manifest entry for it means.
    ///
    /// The hash comes from `Blobs::describe`, which is `iroh-blobs` hashing while
    /// it imports. A test that wrote `blake3::hash(bytes)` would be a test that
    /// pulled a hash function into this crate's graph to check an assertion about
    /// a crate whose design rule is that it does not have one, and the rule in
    /// `lib.rs` has no "but only in tests" clause.
    async fn holds(&self, rel_path: &str, bytes: &[u8]) -> Blob {
        let absolute = self.root().join(rel_path);
        std::fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        std::fs::write(&absolute, bytes).unwrap();
        self.blobs
            .describe(rel_path)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{rel_path} was not made servable"))
    }

    /// The same, for an imported reference — which is the common case and the
    /// one whose path cannot be chosen freely.
    ///
    /// `assets/originals/<hh>/<hash>.<ext>` is named after its own content, and
    /// `blobs::agrees` refuses a pairing where it is not, so the hash has to be
    /// known before the path exists. That ordering is the layout's, not the
    /// test's: `wobu_store::assets` files an import the same way round.
    async fn holds_original(&self, bytes: &[u8]) -> Blob {
        let hash = hash_of(bytes).await;
        self.holds(&format!("assets/originals/{}/{hash}.png", &hash[..2]), bytes).await
    }
}

/// The BLAKE3 of some bytes, obtained by asking `iroh-blobs` for it.
///
/// A throwaway store in a throwaway directory, so nothing about the machine
/// under test is disturbed. Written this way rather than as `blake3::hash`
/// because that would mean pulling a hash function into this crate's graph to
/// check an assertion about a crate whose whole design rule is that it does not
/// have one — and `lib.rs` states that rule without a "but only in tests"
/// clause.
async fn hash_of(bytes: &[u8]) -> String {
    let scratch = Scratch::new();
    // Under `assets/thumbs/`, which is the one syncable tree whose filename is
    // not derived from its own content and therefore the only one that will
    // accept a file before its hash is known.
    let rel_path = "assets/thumbs/00/probe.bin";
    let absolute = scratch.root().join(rel_path);
    std::fs::create_dir_all(absolute.parent().unwrap()).unwrap();
    std::fs::write(&absolute, bytes).unwrap();

    let prober = Blobs::open(scratch.root(), scratch.cache()).await.unwrap();
    let hash = prober.describe(rel_path).await.unwrap().expect("a readable file").hash;
    prober.shutdown().await.unwrap();
    hash
}

/// Two machines holding the same project, connected, with the session as each
/// side sees it.
///
/// Blobs do not need a session — they get their own connection on their own ALPN
/// — but a real sync has one, and `Session::addr` is how the fetching side
/// learns where to dial. Both halves are returned because a manifest exchange
/// needs the same connection from both ends.
async fn pair() -> (Machine, Machine, Session, Session) {
    let project = new_id();
    let mut server = Machine::new(project).await;
    let client = Machine::new(project).await;

    let outbound =
        client.endpoint.connect(server.endpoint.addr(), project).await.expect("both hold it");
    let inbound = server.inbox.recv().await.expect("the accepting side saw the session");
    (server, client, inbound, outbound)
}

/// Generous: every one of these is a loopback transfer, and the tests that care
/// about a deadline set their own.
const PER_BLOB: Duration = Duration::from_secs(30);

/// Anything at or above this is referenced in place by the store rather than
/// inlined into its database — `iroh-blobs`' threshold is 16 KiB. Tests that
/// want the referencing path have to clear it.
const REFERENCED: usize = 64 * 1024;

/* ── the round trip ───────────────────────────────────────────────────────── */

/// The whole feature: bytes that exist on one machine and not the other end up
/// on the other, at the path the manifest named, identical.
#[tokio::test]
async fn a_blob_crosses_the_wire_and_lands_byte_for_byte() {
    let (server, client, _server_session, session) = pair().await;
    let content: Vec<u8> = (0..REFERENCED).map(|n| (n % 251) as u8).collect();
    let blob = server.holds_original(&content).await;
    let rel_path = blob.rel_path.clone();

    let fetched = client
        .blobs
        .fetch(client.endpoint.endpoint(), session.addr(), &[blob], PER_BLOB)
        .await
        .expect("the peer was reachable");

    assert_eq!(fetched.placed, vec![rel_path.clone()]);
    assert_eq!(fetched.failed, 0);
    assert_eq!(fetched.refused, 0);
    assert_eq!(std::fs::read(client.root().join(&rel_path)).unwrap(), content);
}

/// A zero-byte file is a file. It has a BLAKE3, it has a path, and a transfer
/// that treated "nothing arrived" and "nothing to send" as the same thing would
/// leave it missing forever — which, because an absence is never a deletion, is
/// a sync that reports success and never converges.
#[tokio::test]
async fn an_empty_blob_is_a_blob_and_not_an_absence() {
    let (server, client, _server_session, session) = pair().await;
    let blob = server.holds_original(b"").await;
    let rel_path = blob.rel_path.clone();

    let fetched = client
        .blobs
        .fetch(client.endpoint.endpoint(), session.addr(), &[blob], PER_BLOB)
        .await
        .unwrap();

    assert_eq!(fetched.placed, vec![rel_path.clone()]);
    let landed = client.root().join(&rel_path);
    assert!(landed.is_file(), "an empty blob left no file at all");
    assert_eq!(std::fs::metadata(&landed).unwrap().len(), 0);
}

/// A blob far larger than any receive window, and larger than the store's inline
/// threshold, so it takes the referencing and streaming paths rather than the
/// one where everything fits in a database row.
///
/// Eight megabytes rather than four gigabytes because this runs on every commit;
/// what it pins is that the transfer is *streamed and reassembled*, and that is
/// the same code at either size.
#[tokio::test]
async fn a_large_blob_arrives_whole() {
    let (server, client, _server_session, session) = pair().await;
    // Not a repeating byte: a chunking bug that dropped or duplicated a block
    // would be invisible in eight megabytes of the same value.
    let content: Vec<u8> =
        (0..8 * 1024 * 1024u32).map(|n| (n.wrapping_mul(2654435761) >> 24) as u8).collect();
    let blob = server.holds_original(&content).await;
    let rel_path = blob.rel_path.clone();

    let fetched = client
        .blobs
        .fetch(client.endpoint.endpoint(), session.addr(), &[blob], PER_BLOB)
        .await
        .unwrap();

    assert_eq!(fetched.placed, vec![rel_path.clone()]);
    let landed = std::fs::read(client.root().join(&rel_path)).unwrap();
    assert_eq!(landed.len(), content.len());
    assert!(landed == content, "eight megabytes came back different");
}

/// Several blobs, one connection, both trees, in one call.
#[tokio::test]
async fn assets_and_generations_travel_together() {
    let (server, client, _server_session, session) = pair().await;
    let generation = "generations/2026-07/01ARZ3NDEKTSV4RRFFQ69G5FAV.json";
    let wanted = vec![
        server.holds_original(b"one").await,
        server.holds_original(&vec![7u8; REFERENCED]).await,
        server.holds(generation, br#"{"id":"01ARZ3NDEKTSV4RRFFQ69G5FAV"}"#).await,
    ];

    let fetched = client
        .blobs
        .fetch(client.endpoint.endpoint(), session.addr(), &wanted, PER_BLOB)
        .await
        .unwrap();

    assert_eq!(fetched.placed.len(), 3, "{fetched:?}");
    assert_eq!(fetched.failed, 0);
    for blob in &wanted {
        assert!(client.root().join(&blob.rel_path).is_file(), "{} did not land", blob.rel_path);
    }
    assert_eq!(std::fs::read(client.root().join(generation)).unwrap(), br#"{"id":"01ARZ3NDEKTSV4RRFFQ69G5FAV"}"#);
}

/// Fetch-if-missing, which is the whole of the decision. A second call moves
/// nothing, and does not have to ask the peer anything to know that.
#[tokio::test]
async fn a_blob_already_here_is_not_fetched_again() {
    let (server, client, _server_session, session) = pair().await;
    let blob = server.holds_original(&vec![9u8; REFERENCED]).await;
    let endpoint = client.endpoint.endpoint();

    let first =
        client.blobs.fetch(endpoint, session.addr(), std::slice::from_ref(&blob), PER_BLOB).await.unwrap();
    let again = client.blobs.fetch(endpoint, session.addr(), &[blob], PER_BLOB).await.unwrap();

    assert_eq!(first.placed.len(), 1);
    assert!(again.placed.is_empty());
    assert_eq!(again.already, 1);
}

/* ── the path a peer chose ────────────────────────────────────────────────── */

/// **The test this file exists for.**
///
/// A peer sends a manifest full of paths designed to reach outside the project
/// folder, the fetching side is handed the whole list, and afterwards nothing
/// anywhere outside the project folder has changed and nothing inside it is
/// outside `assets/` or `generations/`.
///
/// The list is handed to [`Blobs::fetch`] directly rather than through
/// `manifest::exchange`, and that is deliberate: the exchange drops most of
/// these already, which would make a wire test pass while proving nothing about
/// the join. The join is what runs one line before a real `rename`, and it has
/// to hold on its own. `no_hostile_path_survives_the_exchange_either` covers the
/// other half.
///
/// Two witnesses are planted outside the project folder — one in the scratch
/// parent, one two levels up — and read back at the end. An assertion that a
/// file "was not created" is weaker than it looks when the path was never
/// writable in the first place, so these are files that *exist* and must be
/// unchanged.
#[tokio::test]
async fn no_path_a_peer_can_write_escapes_the_project_folder() {
    let (server, client, _server_session, session) = pair().await;
    let real = server.holds_original(b"the one legitimate file").await;

    // Two things outside the project folder that would be visibly damaged.
    let sibling = client.root().parent().unwrap().join("witness");
    std::fs::write(&sibling, b"untouched").unwrap();
    let deeper = client.root().parent().unwrap().parent().unwrap().join("wobu-sync-witness");
    std::fs::write(&deeper, b"untouched").unwrap();

    let hostile: Vec<Blob> = [
        "../witness",
        "../../wobu-sync-witness",
        "assets/../../witness",
        "assets/originals/../../../wobu-sync-witness",
        "assets/../../../../../../../../tmp/wobu-sync-witness",
        "/tmp/wobu-sync-witness",
        "/etc/passwd",
        "C:\\Windows\\win.ini",
        "\\\\?\\C:\\Windows\\win.ini",
        "\\\\server\\share\\witness",
        "assets\\..\\..\\witness",
        "assets/originals/ab\\..\\..\\..\\witness",
        "assets//../witness",
        "assets/./../witness",
        "assets/originals/ab/x.png:Zone.Identifier",
        "assets/originals/ab/NUL",
        "assets/originals/ab/con.png",
        "assets/originals/ab/x.png.",
        "assets/originals/ab/x.png ",
        "assets/\u{ff0f}..\u{ff0f}witness",
        "assets/\u{2024}\u{2024}/witness",
        "assets/\u{202e}ssentiw/x.png",
        "nodes/character/kael-vantris.md",
        "project.json",
        ".wobu/index.sqlite",
        ".wobu/tmp/x.part",
        "",
    ]
    .iter()
    .map(|rel_path| Blob { rel_path: (*rel_path).to_string(), hash: real.hash.clone() })
    .collect();
    let refused_count = hostile.len();

    let mut wanted = hostile;
    wanted.push(real.clone());
    let fetched = client
        .blobs
        .fetch(client.endpoint.endpoint(), session.addr(), &wanted, PER_BLOB)
        .await
        .unwrap();

    // Every hostile path refused, and the one good one still landed: a check
    // that stopped the whole fetch would pass this test's first half and break
    // sync for everybody with one corrupt index row.
    assert_eq!(fetched.refused, refused_count, "{fetched:?}");
    assert_eq!(fetched.placed, vec![real.rel_path.clone()]);

    assert_eq!(std::fs::read(&sibling).unwrap(), b"untouched");
    assert_eq!(std::fs::read(&deeper).unwrap(), b"untouched");
    let _ = std::fs::remove_file(&deeper);

    // And nothing landed inside the project folder that is not under one of the
    // two trees this crate carries.
    for entry in walk(&client.root()) {
        let rel = entry.strip_prefix(client.root()).unwrap().to_string_lossy().replace('\\', "/");
        assert!(
            rel.starts_with("assets/") || rel.starts_with("generations/") || rel.starts_with(".wobu/"),
            "{rel} appeared in the project folder"
        );
    }
}

/// The other half: the same paths, sent as a real manifest over a real
/// connection, never reach a caller in the first place.
///
/// Belt and braces on purpose. `manifest::is_syncable_rel_path` is the outer
/// check and `blobs::place` is the inner one, and each is written as if the
/// other did not exist — see the argument in `manifest.rs`, which is that a
/// check performed in a different module is a check that stops being performed
/// the day somebody adds a second caller.
#[tokio::test]
async fn no_hostile_path_survives_the_exchange_either() {
    let (_server, _client, server_session, session) = pair().await;
    let hash = "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262".to_string();
    let theirs: Vec<Blob> = ["../../etc/passwd", "assets\\..\\x.png", "/etc/passwd", "nodes/x.md"]
        .iter()
        .map(|rel_path| Blob { rel_path: (*rel_path).to_string(), hash: hash.clone() })
        .collect();

    let (ours, _) = tokio::join!(
        wobu_sync::manifest::exchange(&session, &[], &[], Duration::from_secs(12)),
        wobu_sync::manifest::exchange(&server_session, &[], &theirs, Duration::from_secs(12)),
    );

    let ours = ours.expect("the exchange completed");
    assert!(ours.blobs.is_empty(), "a hostile path reached a caller: {:?}", ours.blobs);
    assert_eq!(ours.refused, theirs.len());
}

/// A path that survives the string rules and is still not somewhere we will
/// write, because the directory in the way is somebody else's symbolic link.
///
/// Reachable on a shared project folder, which is the arrangement Wobu is built
/// around: a lexical check proves the string stays under the root and proves
/// nothing at all about the directories.
#[tokio::test]
#[cfg(unix)]
async fn a_symlinked_directory_in_the_project_folder_is_not_followed() {
    let (server, client, _server_session, session) = pair().await;
    let blob = server.holds_original(b"content").await;

    // Somebody points `assets/originals` at a directory outside the project.
    let elsewhere = client.root().parent().unwrap().join("outside");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("canary"), b"untouched").unwrap();
    std::fs::create_dir_all(client.root().join("assets")).unwrap();
    let _ = std::fs::remove_dir_all(client.root().join("assets/originals"));
    std::os::unix::fs::symlink(&elsewhere, client.root().join("assets/originals")).unwrap();

    let fetched = client
        .blobs
        .fetch(client.endpoint.endpoint(), session.addr(), &[blob], PER_BLOB)
        .await
        .unwrap();

    assert_eq!(fetched.refused, 1, "{fetched:?}");
    assert!(fetched.placed.is_empty());
    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 1, "something was written through the link");
    assert_eq!(std::fs::read(elsewhere.join("canary")).unwrap(), b"untouched");
}

/// The reasons, checked one at a time, without a network.
///
/// The end-to-end test above asserts that nothing escapes. This asserts *which
/// rule* stopped each one, which is what keeps a green suite from meaning "the
/// length check caught everything".
#[test]
fn each_kind_of_hostile_path_is_refused_by_the_rule_that_names_it() {
    let scratch = Scratch::new();
    let root = scratch.root();

    for (path, why) in [
        ("../../etc/passwd", Unplaceable::NotSyncable),
        ("/etc/passwd", Unplaceable::NotSyncable),
        ("assets/../../x", Unplaceable::NotSyncable),
        ("assets\\..\\x.png", Unplaceable::NotSyncable),
        ("\\\\?\\C:\\x", Unplaceable::NotSyncable),
        ("assets/x.png:s", Unplaceable::NotSyncable),
        ("assets/\u{2024}\u{2024}/x", Unplaceable::NotSyncable),
        ("nodes/x.md", Unplaceable::NotSyncable),
        ("assets/originals/ab/x.png.", Unplaceable::TrailingDotOrSpace),
        ("assets/originals/ab/NUL", Unplaceable::ReservedDeviceName),
    ] {
        assert_eq!(wobu_sync::blobs::place(&root, path), Err(why), "{path:?}");
    }
}

/* ── a provider that is wrong ─────────────────────────────────────────────── */

/// **The hash-mismatch test.** A provider whose file changed under it cannot
/// pass off the new bytes, and the receiver ends with no file rather than a
/// wrong one.
///
/// This is the only way to build a lying provider, and it is not contrived: the
/// store references files in place, so a project folder on a share where
/// somebody replaced `assets/originals/ab/<hash>.png` is exactly this. The
/// receiver asked for a hash; `iroh-blobs` verifies the BLAKE3 tree as the bytes
/// arrive; the bytes stop matching and the transfer fails where it is rather
/// than completing and leaving verification to somebody who might forget.
///
/// Note what the receiving side does *not* do about it: it does not hash the
/// result, because it never gets one. That is the argument in `blobs.rs` for the
/// dependency, tested rather than asserted.
#[tokio::test]
async fn a_referenced_file_that_changed_under_the_provider_cannot_be_passed_off() {
    let (server, client, _server_session, session) = pair().await;
    // Above the inline threshold, so the store references it rather than copying
    // it into the database — otherwise the swap below changes nothing.
    let blob = server.holds_original(&vec![1u8; REFERENCED]).await;
    let rel_path = blob.rel_path.clone();

    // The swap: same path, same length, different bytes. The store still
    // believes it holds the original hash.
    std::fs::write(server.root().join(&rel_path), vec![2u8; REFERENCED]).unwrap();

    let fetched = client
        .blobs
        .fetch(client.endpoint.endpoint(), session.addr(), &[blob], Duration::from_secs(20))
        .await
        .unwrap();

    assert_eq!(fetched.failed, 1, "corrupt content was accepted: {fetched:?}");
    assert!(fetched.placed.is_empty());
    assert!(
        !client.root().join(&rel_path).exists(),
        "a file was written for content that never verified"
    );
    assert!(nothing_staged(&client.root()), "a partial file was left behind");
}

/// **The poisoning attack, and the reason `blobs::agrees` exists.**
///
/// BLAKE3 of the empty input is a hash every blob store can satisfy *locally*,
/// without asking anybody anything, because a zero-length blob is complete the
/// moment it is requested. So a peer that announces
/// `("assets/originals/<hh>/<hash of a real asset>.png", <hash of nothing>)`
/// would — with the path check alone — have this module write a verified,
/// correct, zero-byte file exactly where that asset belongs. Every later sync
/// would see a file at that path, conclude it was finished, and never fetch the
/// real one. One permanently broken asset, from one line of manifest, with no
/// error anywhere and no peer left to blame.
///
/// It was found by a test that did not mean to find it: an earlier version of
/// `a_blob_the_peer_cannot_produce_fails_alone` used the empty hash as a
/// stand-in for "a hash nobody has", and the blob arrived.
#[tokio::test]
async fn a_hash_that_does_not_match_the_path_it_names_is_refused() {
    let (server, client, _server_session, session) = pair().await;
    let real = server.holds_original(b"somebody's actual reference image").await;
    let nothing = hash_of(b"").await;

    let poisoned = Blob { rel_path: real.rel_path.clone(), hash: nothing };
    let fetched = client
        .blobs
        .fetch(client.endpoint.endpoint(), session.addr(), &[poisoned], PER_BLOB)
        .await
        .unwrap();

    assert_eq!(fetched.refused, 1, "{fetched:?}");
    assert!(fetched.placed.is_empty());
    assert!(
        !client.root().join(&real.rel_path).exists(),
        "an empty file was written where a real asset lives"
    );

    // And the path is still fetchable afterwards, which is the half that would
    // have been lost: a poisoned path is one no later sync would ever revisit.
    let fetched = client
        .blobs
        .fetch(client.endpoint.endpoint(), session.addr(), std::slice::from_ref(&real), PER_BLOB)
        .await
        .unwrap();
    assert_eq!(fetched.placed, vec![real.rel_path.clone()]);
    assert_eq!(
        std::fs::read(client.root().join(&real.rel_path)).unwrap(),
        b"somebody's actual reference image"
    );
}

/// A peer that announced something it cannot produce costs one failed entry and
/// nothing else. Ordinary rather than hostile: a manifest is a snapshot, and a
/// file can be gone by the time anybody asks for it.
#[tokio::test]
async fn a_blob_the_peer_cannot_produce_fails_alone() {
    let (server, client, _server_session, session) = pair().await;
    let good = server.holds_original(b"here").await;
    // Never written and never offered by anybody: a hash the provider has no
    // idea about, at the path that content would legitimately live at. Not the
    // hash of nothing — that one every store can satisfy locally without asking
    // a peer, which is what `a_hash_that_does_not_match_the_path_it_names_is_refused`
    // is about.
    let absent = hash_of(b"written on no machine anywhere").await;
    let phantom = Blob {
        rel_path: format!("assets/originals/{}/{absent}.png", &absent[..2]),
        hash: absent,
    };

    let fetched = client
        .blobs
        .fetch(
            client.endpoint.endpoint(),
            session.addr(),
            &[phantom.clone(), good.clone()],
            Duration::from_secs(20),
        )
        .await
        .unwrap();

    assert_eq!(fetched.failed, 1, "{fetched:?}");
    assert_eq!(fetched.placed, vec![good.rel_path], "one missing blob stopped the rest");
    assert!(!client.root().join(&phantom.rel_path).exists());
}

/* ── a transfer that stops half way ───────────────────────────────────────── */

/// A fetch that is cut off part way leaves **nothing a reader can see**: no
/// truncated asset, no zero-length placeholder, nothing at the target path at
/// all.
///
/// The failure this rules out is quiet and permanent. `Blobs::fetch` treats a
/// file that exists as a file that is finished — `assets/` is content-addressed
/// and `generations/` is write-once, so there is no third state — so a truncated
/// file at a real path would be skipped by every future sync and the asset would
/// be broken forever, with no error anywhere.
///
/// Cut by giving the transfer a deadline it cannot meet, which drops the future
/// mid-stream. That is the same shape as a cancelled sync, a closed laptop and a
/// quit.
#[tokio::test]
async fn a_transfer_cut_off_part_way_leaves_nothing_a_reader_can_see() {
    let (server, client, _server_session, session) = pair().await;
    let content: Vec<u8> = (0..16 * 1024 * 1024u32).map(|n| (n % 253) as u8).collect();
    let blob = server.holds_original(&content).await;
    let rel_path = blob.rel_path.clone();

    let cut = client
        .blobs
        .fetch(
            client.endpoint.endpoint(),
            session.addr(),
            &[blob],
            // The connection is already open by the time this deadline starts,
            // so five milliseconds is several hundred kilobytes into the stream
            // on loopback — and sixteen megabytes measures at a couple of
            // hundred milliseconds here, so the margin is fifty-fold rather than
            // twofold. A tight deadline on a small blob would make this a test
            // of whether the machine was busy.
            Duration::from_millis(5),
        )
        .await
        .unwrap();

    assert_eq!(cut.failed, 1, "the transfer finished; raise the size or lower the deadline");
    assert!(cut.placed.is_empty());
    assert!(!client.root().join(&rel_path).exists(), "a partial asset is visible at its real path");
    assert!(nothing_staged(&client.root()), "a `.part` outlived the transfer that made it");
}

/// The same interruption, one layer down: the whole `fetch` future is dropped
/// rather than an inner deadline expiring. Cancellation is a documented property
/// of [`Blobs::fetch`] and a dropped future takes a different path out of the
/// export than a timeout does.
#[tokio::test]
async fn dropping_a_fetch_leaves_the_project_folder_alone() {
    let (server, client, _server_session, session) = pair().await;
    let content: Vec<u8> = (0..16 * 1024 * 1024u32).map(|n| (n % 251) as u8).collect();
    let blob = server.holds_original(&content).await;
    let rel_path = blob.rel_path.clone();

    let wanted = [blob];
    let fetch = client.blobs.fetch(client.endpoint.endpoint(), session.addr(), &wanted, PER_BLOB);
    let dropped = tokio::time::timeout(Duration::from_millis(30), fetch).await;

    assert!(dropped.is_err(), "the transfer finished; raise the size or lower the deadline");
    assert!(!client.root().join(&rel_path).exists(), "a partial asset is visible at its real path");
}

/* ── what a machine will serve ────────────────────────────────────────────── */

/// What is offered is what is on disk, and a list that disagrees with the disk
/// is reported rather than papered over.
///
/// Three cases, and the middle one is the one that matters: a file whose content
/// no longer matches the hash the caller announced is **not offered**, because
/// serving it under a hash it does not have is impossible and serving it under
/// its real hash is answering a question nobody asked.
#[tokio::test]
async fn a_list_that_disagrees_with_the_disk_is_reported_and_not_papered_over() {
    let machine = Machine::new(new_id()).await;
    let real = machine.holds_original(&vec![3u8; REFERENCED]).await;

    // Named in the list, absent from the disk: an index that is ahead of the
    // filesystem, which is ordinary on a share still catching up.
    let never = hash_of(b"never written down").await;
    let missing =
        Blob { rel_path: format!("assets/originals/{}/{never}.png", &never[..2]), hash: never };

    // Present, and not what the list says it is. Note that its *path* is
    // consistent with the announced hash — this is a stale index and not a bad
    // pairing, and the two are counted separately on purpose.
    let claimed = hash_of(b"what the index believes").await;
    let stale_path = format!("assets/originals/{}/{claimed}.png", &claimed[..2]);
    std::fs::create_dir_all(machine.root().join(&stale_path).parent().unwrap()).unwrap();
    std::fs::write(machine.root().join(&stale_path), b"but the file says otherwise").unwrap();
    let stale = Blob { rel_path: stale_path, hash: claimed };

    let hostile = Blob { rel_path: "../../etc/passwd".into(), hash: real.hash.clone() };

    let offered = machine.blobs.offer(&[real, missing, stale, hostile]).await.unwrap();

    // `already`, not `offered`: `Machine::holds` imported it once already, and a
    // second call must not re-read the file.
    assert_eq!(offered.already, 1, "{offered:?}");
    assert_eq!(offered.missing, 1, "{offered:?}");
    assert_eq!(offered.stale, 1, "{offered:?}");
    assert_eq!(offered.refused, 1, "{offered:?}");
    assert_eq!(offered.offered, 0, "{offered:?}");
}

/// The store is derived state and must not live in the folder two machines have
/// mounted.
///
/// `wobu_store::paths::index_path` keeps the SQLite index in local app data for
/// exactly this reason. A redb database inside a Dropbox folder is two processes
/// writing one file over a network filesystem, and the failure is corruption
/// rather than an error — which is why this is refused at `open` rather than
/// documented.
#[tokio::test]
async fn a_blob_store_inside_the_project_folder_is_refused() {
    let scratch = Scratch::new();

    let inside = Blobs::open(scratch.root(), scratch.root().join(".wobu/blobs")).await;
    let also_inside = Blobs::open(scratch.root(), scratch.root().join("..").join("project").join("x")).await;
    let outside = Blobs::open(scratch.root(), scratch.cache()).await;

    assert!(inside.is_err(), "a store inside the project folder was allowed");
    assert!(also_inside.is_err(), "a `..` walked straight past the check");
    assert!(outside.is_ok(), "{:?}", outside.err());
    outside.unwrap().shutdown().await.unwrap();
}

/// A project root that is not there is an error rather than a directory this
/// crate creates. Creating one would mean a transport crate deciding a project
/// exists, which is `wobu-store`'s to say.
#[tokio::test]
async fn a_project_root_that_is_not_there_is_refused() {
    let scratch = Scratch::new();

    let gone = Blobs::open(scratch.root().join("nope"), scratch.cache()).await;

    assert!(gone.is_err());
}

/* ── helpers ──────────────────────────────────────────────────────────────── */

/// Every file under a directory, recursively. `walkdir` is a dependency this
/// crate does not have and this is nine lines.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return found };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(walk(&path));
        } else {
            found.push(path);
        }
    }
    found
}

/// Whether the staging directory is clear.
///
/// A `.part` left behind is litter rather than damage — `Project::sweep` deletes
/// them on open — but a `.part` left behind *by a path that completed* would
/// mean the cleanup is only running on the happy path, which is the one that
/// does not need it.
fn nothing_staged(root: &Path) -> bool {
    match std::fs::read_dir(root.join(".wobu/tmp")) {
        Err(_) => true,
        Ok(entries) => entries.flatten().count() == 0,
    }
}
