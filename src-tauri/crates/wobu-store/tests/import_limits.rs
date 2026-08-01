//! What an import refuses, what it warns about, and what it leaves alone (#30).
//!
//! `assets.rs` covers the property content addressing exists to buy. This file
//! covers the edges around it: the pictures that arrive at the drop target and
//! are not, in one way or another, a reference image Wobu can put in a folder
//! and hand to a provider.
//!
//! The division under test is the one `wobu_store::assets`' module docs argue
//! for, and it is the whole point of the issue:
//!
//! - **Refused** is a fact about the folder, true on every machine — bytes no
//!   parser recognises, and files holding more than one frame. Nothing is
//!   written and the error says what to do instead.
//! - **Warned about** is a fact about one provider — the bounds Hunyuan3D
//!   states in `docs/08-providers.md`. The picture lands, and the outcome says
//!   what a later stage will object to, because the alternative is a paid call
//!   failing minutes later with nothing on screen connecting it to a drag from
//!   Tuesday.
//!
//! Plus the two that are neither: a photograph is recorded the way up it is
//! meant to be seen, and an import of a large file can be stopped.

use std::fs;

use wobu_core::AssetKind;
use wobu_store::assets::{MESH_MAX_BYTES, MESH_MAX_SIDE, MESH_MIN_SIDE};
use wobu_store::{Cancel, Error, ImportWarning, Project};

/* ── fixtures ─────────────────────────────────────────────────────────────── */

/// A whole PNG — signature, IHDR, the chunks a test is about, then IDAT and
/// IEND. Whole rather than header-only because the frame count is read off the
/// chunk list, and a bare header settles nothing.
fn png(width: u32, height: u32) -> Vec<u8> {
    png_with(width, height, &[])
}

fn png_with(width: u32, height: u32, ancillary: &[Vec<u8>]) -> Vec<u8> {
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    out.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    for chunk in ancillary {
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&png_chunk(b"IDAT", &[0; 8]));
    out.extend_from_slice(&png_chunk(b"IEND", &[]));
    out
}

fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = (payload.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out.extend_from_slice(&[0, 0, 0, 0]);
    out
}

/// An APNG: the same PNG with the animation control chunk that makes it one.
fn apng(width: u32, height: u32) -> Vec<u8> {
    png_with(width, height, &[png_chunk(b"acTL", &[0, 0, 0, 4, 0, 0, 0, 0])])
}

/// SOI, an APP0 to walk past, then SOF0.
fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut out = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10];
    out.extend_from_slice(b"JFIF\0");
    out.extend_from_slice(&[0; 9]);
    out.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    out
}

/// A JPEG out of a phone: pixels stored landscape, an EXIF block beside them
/// saying which way to turn them, and a GPS tag riding along behind it.
fn photograph(width: u16, height: u16, orientation: u16) -> Vec<u8> {
    let mut exif = b"Exif\0\0II\x2a\x00".to_vec();
    exif.extend_from_slice(&8u32.to_le_bytes());
    exif.extend_from_slice(&2u16.to_le_bytes());
    for (tag, value) in [(0x0112u16, orientation), (0x8825, 0x4242)] {
        exif.extend_from_slice(&tag.to_le_bytes());
        exif.extend_from_slice(&3u16.to_le_bytes());
        exif.extend_from_slice(&1u32.to_le_bytes());
        exif.extend_from_slice(&value.to_le_bytes());
        exif.extend_from_slice(&[0, 0]);
    }
    exif.extend_from_slice(&0u32.to_le_bytes());

    let mut out = vec![0xff, 0xd8, 0xff, 0xe1];
    out.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&exif);
    out.extend_from_slice(&jpeg(width, height)[2..]);
    out
}

fn gif_header(width: u16, height: u16) -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&[0x00, 0x00, 0x00]);
    out
}

/// A GIF with `frames` image descriptors — an animation at two and above, and
/// byte-identical to a still for its first thirteen bytes.
fn gif(width: u16, height: u16, frames: usize) -> Vec<u8> {
    let mut out = gif_header(width, height);
    for _ in 0..frames {
        out.extend_from_slice(&[0x21, 0xf9, 0x04, 0x00, 0x0a, 0x00, 0x00, 0x00]);
        out.extend_from_slice(&[0x2c, 0, 0, 0, 0]);
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&[0x00, 0x02, 0x00]);
    }
    out.push(0x3b);
    out
}

/// An extended WebP, `flags` being the byte that says whether it animates.
fn webp(width: u32, height: u32, flags: u8) -> Vec<u8> {
    let mut payload = vec![flags, 0, 0, 0];
    payload.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    payload.extend_from_slice(&(height - 1).to_le_bytes()[..3]);

    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((payload.len() + 12) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(b"VP8X");
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// A multi-page TIFF, little-endian, with a second IFD offset on the first.
fn tiff() -> Vec<u8> {
    let mut out = b"II\x2a\x00".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&20u32.to_le_bytes());
    out.extend_from_slice(&[0; 32]);
    out
}

fn new_project() -> (tempfile::TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::create(dir.path(), "Ashfall").unwrap();
    (dir, project)
}

fn blobs(project: &Project) -> usize {
    wobu_store::assets::scan(project.root()).len()
}

/* ── more than one frame is refused ───────────────────────────────────────── */

#[test]
fn an_animated_gif_is_refused_rather_than_stored_as_whichever_frame_came_first() {
    // The "pick a frame or refuse" of #30, answered. Taking frame one would
    // make the asset depend on a choice nothing on screen records, and would
    // put a file in the library whose thumbnail is a still of something the
    // user thinks is moving.
    let (_dir, mut project) = new_project();

    let outcome = project.import_asset(&gif(320, 200, 12), AssetKind::Reference);
    assert!(matches!(outcome, Err(Error::AnimatedImage)), "{outcome:?}");

    assert_eq!(blobs(&project), 0, "an animation was written anyway");
    assert!(project.list_assets().unwrap().is_empty());
}

#[test]
fn an_apng_and_an_animated_webp_are_refused_on_exactly_the_same_terms() {
    // Total over every format Wobu reads, which is what "don't half-support
    // it" asks for. A rule that caught GIF and let an APNG through would be
    // the same bug with a different extension on it.
    let (_dir, mut project) = new_project();

    for bytes in [apng(400, 400), webp(400, 400, 0x02)] {
        let outcome = project.import_asset(&bytes, AssetKind::Reference);
        assert!(matches!(outcome, Err(Error::AnimatedImage)), "{outcome:?}");
    }
    assert_eq!(blobs(&project), 0);
}

#[test]
fn a_still_of_each_of_those_formats_imports_normally() {
    // The other half, and the one that makes the rule about frames rather than
    // about formats: refusing every GIF would be total too, and would throw
    // away a perfectly good reference image.
    let (_dir, mut project) = new_project();

    for bytes in [gif(320, 200, 1), png(400, 400), webp(400, 400, 0x10)] {
        let imported = project.import_asset(&bytes, AssetKind::Reference).unwrap();
        assert!(!imported.deduped);
    }
    assert_eq!(project.list_assets().unwrap().len(), 3);
}

#[test]
fn a_half_copied_gif_is_not_mistaken_for_an_animation() {
    // The distinction the frame count is built on. A file that stops early is
    // a file a sync client is still copying, and refusing every one of them
    // would refuse pictures that are perfectly fine a second later — which is
    // a worse failure than the one the animation rule exists to prevent,
    // because it happens on ordinary files rather than on unusual ones.
    let (_dir, mut project) = new_project();
    let full = gif(320, 200, 2);

    let imported = project.import_asset(&full[..20], AssetKind::Reference).unwrap();
    assert_eq!((imported.asset.width, imported.asset.height), (320, 200));

    // And the whole file, once it has finished copying, is refused.
    let outcome = project.import_asset(&full, AssetKind::Reference);
    assert!(matches!(outcome, Err(Error::AnimatedImage)), "{outcome:?}");
}

#[test]
fn an_animation_a_person_dropped_into_the_folder_by_hand_stays_out_of_the_library() {
    // The index has to describe the library an import would have produced.
    // Indexing this would put an asset in the library that re-importing the
    // same picture rejects, and nothing on screen could explain the pair.
    let (_dir, mut project) = new_project();
    let hash = "c".repeat(64);
    let shard = project.root().join("assets/originals").join(&hash[..2]);
    fs::create_dir_all(&shard).unwrap();
    fs::write(shard.join(format!("{hash}.gif")), gif(64, 64, 4)).unwrap();

    project.rebuild_index().unwrap();

    assert!(project.list_assets().unwrap().is_empty());
    // And the file itself is left exactly where it is — being unindexable is
    // not a licence to delete somebody's file.
    assert!(shard.join(format!("{hash}.gif")).is_file());
}

#[test]
fn a_multi_page_tiff_is_refused_as_a_format_rather_than_as_a_page_count() {
    // One rule, no page count involved: there is no TIFF parser here, so a
    // `.tif` never reaches the question of which page was meant. The message
    // is the one that lists the formats that do work, which is the only thing
    // the user can act on.
    let (_dir, mut project) = new_project();

    let outcome = project.import_asset(&tiff(), AssetKind::Reference);
    assert!(matches!(outcome, Err(Error::NotAnImage)), "{outcome:?}");
    assert!(outcome.unwrap_err().to_string().contains("PNG, JPEG, GIF and WebP"));
    assert_eq!(blobs(&project), 0);
}

#[test]
fn the_two_refusals_do_not_share_a_sentence() {
    // They need opposite advice. "That is not an image Wobu can read" told to
    // somebody holding an animated GIF is both wrong and unactionable — they
    // can see it perfectly well in their file browser.
    let (_dir, mut project) = new_project();
    let animation =
        project.import_asset(&gif(64, 64, 2), AssetKind::Reference).unwrap_err().to_string();
    let rubbish =
        project.import_asset(b"%PDF-1.7\n", AssetKind::Reference).unwrap_err().to_string();

    assert_ne!(animation, rubbish);
    assert!(animation.contains("frame"), "{animation}");
}

/* ── the 3D bounds are a warning, not a refusal ───────────────────────────── */

#[test]
fn an_image_outside_the_3d_bounds_is_imported_and_the_caller_is_told_why() {
    // The whole "warn rather than silently accept" of #30. The picture is a
    // fine mood reference and may never go near a mesh, so refusing it makes
    // a decision the user has not been asked to make — but accepting it in
    // silence spends a paid call to discover the same thing at submit time.
    let (_dir, mut project) = new_project();

    let small = project.import_asset(&png(64, 64), AssetKind::Reference).unwrap();
    assert_eq!(small.warnings, [ImportWarning::MeshTooSmall]);
    assert!(!small.deduped);

    let large = project.import_asset(&png(6000, 4000), AssetKind::Reference).unwrap();
    assert_eq!(large.warnings, [ImportWarning::MeshTooLarge]);

    // Both are in the library, indexed, exactly like anything else.
    assert_eq!(project.list_assets().unwrap().len(), 2);
    assert_eq!(blobs(&project), 2);
}

#[test]
fn an_image_inside_every_bound_carries_no_warnings_at_all() {
    // The bounds are inclusive at both ends, and an empty list is what the UI
    // reads as "nothing to say" — a warning on a picture that is fine teaches
    // people to ignore the ones that are not.
    let (_dir, mut project) = new_project();

    for bytes in [png(MESH_MIN_SIDE, MESH_MAX_SIDE), jpeg(1024, 768)] {
        let imported = project.import_asset(&bytes, AssetKind::Reference).unwrap();
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
    }
}

#[test]
fn a_gif_or_a_webp_is_stored_happily_and_flagged_as_unusable_for_multi_view() {
    // Hunyuan3D's multi-view input is JPG/PNG only (docs/08-providers.md).
    // That is a fact about one capability, not about the picture, so the name
    // of the warning says so and the import goes ahead.
    let (_dir, mut project) = new_project();

    for bytes in [gif(512, 512, 1), webp(512, 512, 0x10)] {
        let imported = project.import_asset(&bytes, AssetKind::Reference).unwrap();
        assert_eq!(imported.warnings, [ImportWarning::MeshFormat]);
    }
    assert_eq!(project.list_assets().unwrap().len(), 2);
}

#[test]
fn a_file_too_heavy_to_send_is_warned_about_even_though_it_is_perfectly_valid() {
    // The multi-view *total* is 6 MB before base64, so one image over that can
    // be in no turnaround sheet at all. Nothing about the image is wrong, and
    // nothing about it will be different when it is submitted, which is why it
    // is worth saying now rather than then.
    let (_dir, mut project) = new_project();
    let mut bytes = png(2048, 2048);
    // Trailing bytes after IEND: a real file's payload, as far as the header
    // parser is concerned, and all this test needs is the length.
    bytes.resize(MESH_MAX_BYTES as usize + 1, 0);

    let imported = project.import_asset(&bytes, AssetKind::Reference).unwrap();
    assert_eq!(imported.warnings, [ImportWarning::MeshTooHeavy]);
    assert_eq!(imported.asset.bytes, MESH_MAX_BYTES + 1);
}

#[test]
fn the_second_person_to_import_a_picture_is_told_the_same_thing_as_the_first() {
    // A warning computed only on a write would mean the collaborator whose
    // import deduped never hears it — and on a share, deduping is the ordinary
    // case rather than the exception.
    let (_dir, mut jake) = new_project();
    let (_hers, mut nadia) = new_project();
    let bytes = png(64, 64);

    let first = jake.import_asset(&bytes, AssetKind::Reference).unwrap();
    let again = jake.import_asset(&bytes, AssetKind::Reference).unwrap();
    let hers = nadia.import_asset(&bytes, AssetKind::Reference).unwrap();

    assert!(again.deduped && !first.deduped);
    assert_eq!(again.warnings, first.warnings);
    assert_eq!(hers.warnings, first.warnings);
    assert_eq!(first.warnings, [ImportWarning::MeshTooSmall]);
}

/* ── orientation ──────────────────────────────────────────────────────────── */

#[test]
fn a_photograph_is_recorded_the_way_up_it_is_meant_to_be_seen() {
    // A phone shoots landscape and writes "rotate 90°" beside the pixels.
    // Recording the stored numbers puts every portrait photo in the library on
    // its side, and every thumbnail and every reference sent to a backend with
    // it.
    let (_dir, mut project) = new_project();

    let upright = project.import_asset(&photograph(4032, 3024, 1), AssetKind::Reference).unwrap();
    assert_eq!((upright.asset.width, upright.asset.height), (4032, 3024));

    let turned = project.import_asset(&photograph(4032, 3024, 6), AssetKind::Reference).unwrap();
    assert_eq!((turned.asset.width, turned.asset.height), (3024, 4032));
}

#[test]
fn the_side_measured_against_the_3d_bounds_is_the_side_the_provider_will_see() {
    // The two decisions have to agree. A 100x4000 scan tagged "rotate 90°" is
    // 4000x100 to anything that honours the tag — still out of bounds, but a
    // build that applied the orientation to the record and not to the check
    // would be measuring a shape nobody ever sees.
    let (_dir, mut project) = new_project();

    let sideways = project.import_asset(&photograph(4000, 100, 6), AssetKind::Reference).unwrap();
    assert_eq!((sideways.asset.width, sideways.asset.height), (100, 4000));
    assert_eq!(sideways.warnings, [ImportWarning::MeshTooSmall]);

    // And a rotation that takes the long side out of bounds is caught as
    // readily as one that leaves it there.
    let tall = project.import_asset(&photograph(6000, 1000, 8), AssetKind::Reference).unwrap();
    assert_eq!((tall.asset.width, tall.asset.height), (1000, 6000));
    assert_eq!(tall.warnings, [ImportWarning::MeshTooLarge]);
}

#[test]
fn the_users_file_reaches_the_folder_byte_for_byte_metadata_and_all() {
    // The decision `image`'s module docs argue: orientation is applied to what
    // Wobu *records*, and the bytes are never rewritten. Stripping the GPS tag
    // or baking the rotation into the pixels would make the hash a function of
    // our encoder as well as of the picture, and two people on two builds
    // would get two files and two ids for one photograph.
    //
    // The consequence is stated rather than hidden: the EXIF block, GPS
    // included, is in the shared folder. Stripping belongs on the way out.
    let (dir, mut project) = new_project();
    let bytes = photograph(4032, 3024, 6);

    let imported = project.import_asset(&bytes, AssetKind::Reference).unwrap();
    let written = fs::read(dir.path().join("ashfall.wobu").join(&imported.asset.rel_path)).unwrap();

    assert_eq!(written, bytes, "the import rewrote the user's file");
    assert!(written.windows(6).any(|w| w == b"Exif\0\0"));
    // Same picture, same hash, whoever imports it and on whichever build.
    assert_eq!(imported.asset.hash, wobu_store::atomic::hash_bytes(&bytes));
}

/* ── a large import can be stopped ────────────────────────────────────────── */

#[test]
fn an_import_that_was_already_cancelled_writes_nothing_and_is_not_a_failure() {
    // `Error::Cancelled` is its own answer for the reason `scan` gives: nothing
    // was written, the folder is as it was, and the UI must not offer to retry
    // something the user just asked to stop.
    let (_dir, mut project) = new_project();
    let cancel = Cancel::new();
    cancel.cancel();

    let outcome = project.import_asset_with(&png(2048, 2048), AssetKind::Reference, &cancel);
    assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");

    assert_eq!(blobs(&project), 0);
    assert!(project.list_assets().unwrap().is_empty());
}

#[test]
fn a_large_file_import_is_stoppable_between_chunks_rather_than_at_the_end() {
    // The half of #30 this crate owns. A 300 MB scan on a share is minutes of
    // reading, and without a way out it is an app that has to be killed from a
    // terminal — the same argument `Project::open_with` is built on, answered
    // with the same token rather than with a second mechanism.
    let (_dir, mut project) = new_project();
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("scan.png");

    let mut bytes = png(4000, 3000);
    bytes.resize(8 * 1024 * 1024, 0);
    fs::write(&path, &bytes).unwrap();

    let cancel = Cancel::new();
    cancel.cancel();
    let outcome = project.import_asset_file_with(&path, AssetKind::Reference, &cancel);
    assert!(matches!(outcome, Err(Error::Cancelled)), "{outcome:?}");
    assert_eq!(blobs(&project), 0);
    assert!(staging_is_empty(&project));

    // The same file, uncancelled, still imports — the token is the only
    // difference between the two calls.
    let imported = project.import_asset_file(&path, AssetKind::Reference).unwrap();
    assert_eq!(imported.asset.bytes, bytes.len() as u64);
    assert_eq!(blobs(&project), 1);
}

fn staging_is_empty(project: &Project) -> bool {
    fs::read_dir(project.root().join(".wobu/tmp")).map(|d| d.count() == 0).unwrap_or(true)
}

#[test]
fn a_cancelled_import_leaves_no_staging_litter_for_the_next_one_to_trip_over() {
    // A `.part` file left behind is not merely untidy: `sweep_staging` runs on
    // open and a stray one is a file Wobu has to reason about every session.
    let (_dir, mut project) = new_project();
    let cancel = Cancel::new();
    cancel.cancel();

    let _ = project.import_asset_with(&png(1024, 1024), AssetKind::Reference, &cancel);

    assert!(staging_is_empty(&project));
}
