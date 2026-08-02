//! Thumbnails, generated into the project folder (#25).
//!
//! # Why they live in the project and not in local app data
//!
//! The obvious place for a thumbnail is a cache under app data, keyed by
//! something, swept when it gets big. That is exactly backwards for Wobu, and
//! the reason is the one `assets` is built on: a thumbnail here is
//! **content-addressed and conflict-free**. It is named after the *original's*
//! hash, at `assets/thumbs/a3/a3f9…c1.webp`, sharded exactly like the blob it
//! describes — so two people on one share cannot produce two thumbnails for one
//! picture, and the first person to import a reference pays the decode for
//! everyone. In a local cache every collaborator would pay it separately, for
//! bytes that are identical on all of their machines.
//!
//! It also survives the thing app data does not: `docs/02-data-model.md`
//! promises the folder is the world, so copying it to a USB stick has to carry
//! the library across, thumbnails and all.
//!
//! # The bytes need not agree between machines, and the path is why
//!
//! `assets` insists the path of a blob is a pure function of its bytes, because
//! two spellings of one picture would break the conflict-free claim. A
//! thumbnail inverts that: its path is a function of *someone else's* bytes,
//! and its own contents depend on our decoder, our downsampler and libwebp's
//! version. Two builds can therefore produce two different files for one path.
//!
//! That is fine, and it is fine for one reason: **a thumbnail that is already
//! there is never rewritten** — see [`ensure`]. So the bytes on the share are
//! whoever got there first, nobody's write ever lands on top of a file another
//! machine is reading, and the worst case is that two Wobus would have drawn
//! marginally different previews of the same picture. Nothing is lost, because
//! nothing here is canonical: delete `assets/thumbs/` entirely and the next
//! grid rebuilds it.
//!
//! # Orientation is applied here, and only here
//!
//! `crate::image` reads the EXIF orientation tag and records the *displayed*
//! dimensions, but the store never rotates a pixel: an import writes the user's
//! file through byte for byte, so that the hash stays a function of the picture
//! rather than of our encoder. That leaves exactly one place in Wobu where the
//! transform can actually be carried out — the one that decodes — and this is
//! it. A thumbnail that ignored the tag would put every phone photograph in the
//! grid on its side while the card beside it reported portrait dimensions.
//!
//! The tag is read once, by [`crate::image::probe`], and mapped onto the
//! decoder's own transform in `turn`. It is deliberately not re-read from the
//! decoder's metadata: two readers of one tag are two chances to disagree about
//! which way up a photograph goes.

use std::io::Cursor;
use std::path::Path;

use image::imageops::FilterType;
use image::{DynamicImage, ImageReader, Limits};

use crate::error::{Error, Result};
use crate::image::Orientation;
use crate::scan::Cancel;
use crate::{assets, paths};

/// Where thumbnails live, relative to the project root.
pub const THUMBS_DIR: &str = "assets/thumbs";

/// The longest side of a generated thumbnail, in pixels.
///
/// 512 rather than the ~180 a grid tile is drawn at, because the same file has
/// to serve the tile, the hover preview and a 2x display without going back to
/// a 40 MB original. Doubling it again roughly quadruples what a share carries
/// for a library, which is the cost this number is really trading against.
pub const MAX_SIDE: u32 = 512;

/// libwebp's quality knob. 82 is the knee of the curve for photographic
/// references at this size — visibly lossless in a grid, and a fraction of what
/// the lossless encoder would write. The originals are untouched and are what
/// anything downstream is actually sent, so nothing is degraded by this except
/// a preview.
const QUALITY: f32 = 82.0;

/// The most a single decode may allocate.
///
/// A header states its own dimensions, so a malformed or hostile file can ask
/// for an arbitrary allocation before a single pixel is read — and thumbnailing
/// is the one path in Wobu that hands somebody else's file to a decoder. Half a
/// gigabyte is comfortably above a 100-megapixel scan at four bytes a pixel and
/// well below "the app disappeared".
const MAX_DECODED_BYTES: u64 = 512 * 1024 * 1024;

/// What a thumbnail request did, as opposed to what it produced.
///
/// The same shape and the same reasoning as [`crate::ImportedAsset`]: the path
/// comes back identical either way, so `generated` is the only thing that says
/// whether anything was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    /// Project-relative, `/`-separated — the value that goes on
    /// `Asset::thumb_path`.
    pub rel_path: String,
    /// False when the file was already in the folder and nothing was written.
    pub generated: bool,
}

/// Where the thumbnail for this hash goes. One definition, in `wobu_core`,
/// beside the one for originals.
pub fn rel_path(hash: &str) -> String {
    wobu_core::asset::thumb_path(hash)
}

/// Whether the folder already holds a thumbnail for this hash.
///
/// A zero-length file reads as absent, because that is what a sync client
/// mid-copy leaves behind and treating it as a hit would put a broken tile in
/// the grid until somebody deleted the file by hand.
pub fn exists(root: &Path, hash: &str) -> bool {
    std::fs::metadata(paths::from_rel_string(root, &rel_path(hash)))
        .is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// Make the thumbnail for one blob, or discover it is already there.
///
/// This is the whole of "missing thumbs are regenerated lazily". Nothing in
/// Wobu may assume a thumbnail exists: a project folder that arrived over sync,
/// out of a zip, or off a USB stick can perfectly well have `assets/originals/`
/// and no `assets/thumbs/` at all, and it has to work. So every reader goes
/// through here, and the *first* one to ask for a picture nobody has drawn yet
/// pays for it — exactly as the first importer does.
///
/// `original_rel` rather than a re-derived path, because the extension is a
/// fact about the bytes that only [`assets`] holds; guessing it here would be a
/// second place that can be wrong about where a blob lives.
///
/// Stoppable through the same token an import and a scan use. The checks sit
/// either side of the read and after the decode — the decode itself is one call
/// into a codec and cannot be interrupted part-way, which bounds a cancel by
/// one image rather than by one library.
pub fn ensure(root: &Path, hash: &str, original_rel: &str, cancel: &Cancel) -> Result<Thumbnail> {
    let rel = rel_path(hash);
    if exists(root, hash) {
        // Not an optimisation. Rewriting a file a collaborator is reading, to
        // replace it with a picture of the same thing, is pure risk for no gain
        // — and it is what makes two builds' differing encoders harmless.
        return Ok(Thumbnail { rel_path: rel, generated: false });
    }

    cancel.check()?;
    let source = paths::from_rel_string(root, original_rel);
    let bytes = assets::read_cancellable(&source, cancel)?;
    cancel.check()?;

    ensure_with_bytes(root, hash, original_rel, &bytes, cancel)
}

/// Make a thumbnail from bytes the caller already owns.
///
/// The import path uses this immediately after publishing the original. Its
/// source file may be hundreds of megabytes away on a share, and reading it a
/// second time merely to decode the same bytes doubles both the latency and the
/// I/O. `original_rel` is retained for an honest decoder error path; it is not
/// opened here.
pub fn ensure_with_bytes(
    root: &Path,
    hash: &str,
    original_rel: &str,
    bytes: &[u8],
    cancel: &Cancel,
) -> Result<Thumbnail> {
    let rel = rel_path(hash);
    if exists(root, hash) {
        return Ok(Thumbnail { rel_path: rel, generated: false });
    }

    cancel.check()?;
    let source = paths::from_rel_string(root, original_rel);

    let webp = render(&source, bytes)?;
    cancel.check()?;

    let path = paths::from_rel_string(root, &rel);
    assets::stage_and_rename(root, &path, &webp)?;
    Ok(Thumbnail { rel_path: rel, generated: true })
}

/// A blob that may be missing its thumbnail: everything [`ensure`] needs, plus
/// the id the result has to be recorded against.
///
/// Carried as data rather than looked up per blob because the whole point of
/// the bulk pass is to run with no lock on the project held — see
/// [`Project::missing_thumbs`](crate::Project::missing_thumbs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbTarget {
    pub asset_id: wobu_core::Id,
    pub hash: String,
    /// The *original's* project-relative path.
    pub rel_path: String,
}

/// Fill in every missing thumbnail in a library, reporting progress.
///
/// This is what a folder that arrived over sync needs: `assets/originals/` full
/// and `assets/thumbs/` absent, which [`ensure`] handles one picture at a time
/// and which somebody has to actually walk.
///
/// Returns every id that **now has** a thumbnail rather than only the ones this
/// pass drew, and the difference is a real case rather than pedantry: a
/// collaborator's Wobu filling in the same folder writes files this machine
/// never made, and the reconcile deliberately does not re-describe a blob whose
/// path it has already seen. Reporting only our own work would leave those rows
/// saying `thumbPath: null` until the index was rebuilt.
///
/// **One blob failing does not stop the pass.** A half-copied file is the
/// ordinary state of a file on a share, and a run that gave up on the first one
/// would leave a thousand-image library with no previews because of one. The
/// skipped blob is simply still missing a thumbnail, which is a state this
/// module is built to be in — the next pass, or the next grid, tries again.
///
/// Cancellation is the one error that *does* stop it, for the reason
/// `rescan_with` gives: the user asked, and every remaining blob is another
/// read off a mount that may be the reason they asked.
pub fn ensure_all(
    root: &Path,
    targets: &[ThumbTarget],
    cancel: &Cancel,
    on_progress: &mut impl FnMut(crate::scan::ScanProgress),
) -> Result<Vec<wobu_core::Id>> {
    let mut present = Vec::new();
    for (done, target) in targets.iter().enumerate() {
        cancel.check()?;
        on_progress(crate::scan::ScanProgress { done, total: targets.len() });

        match ensure(root, &target.hash, &target.rel_path, cancel) {
            Ok(_) => present.push(target.asset_id),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(_) => {}
        }
    }
    on_progress(crate::scan::ScanProgress { done: targets.len(), total: targets.len() });
    Ok(present)
}

/// Decode, turn the right way up, shrink, encode.
///
/// Split out from [`ensure`] so the pixel work is testable without a folder,
/// and so the one function that touches a decoder has no filesystem in it.
fn render(path: &Path, bytes: &[u8]) -> Result<Vec<u8>> {
    // From our own header parser, not from the decoder's metadata. See the
    // module docs: one read of the tag, one answer.
    let orientation = crate::image::probe(bytes).map(|i| i.orientation).unwrap_or_default();

    let decoded = turn(decode(path, bytes)?, orientation);
    let (width, height) = (decoded.width(), decoded.height());
    // Never upscaled. A 64px sprite blown up to 512 is a bigger file than the
    // original conveying no more of it, and the grid can draw a small tile
    // small.
    let scaled = if width.max(height) > MAX_SIDE {
        // `resize` rather than `thumbnail`: the cheap path box-filters, which
        // is fine for a photograph and turns a line-art reference — which is
        // most of what a concept artist collects — into aliased mush.
        decoded.resize(MAX_SIDE, MAX_SIDE, FilterType::CatmullRom)
    } else {
        decoded
    };
    Ok(encode(&scaled))
}

fn decode(path: &Path, bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| Error::io(path, e))?;
    let mut limits = Limits::no_limits();
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);

    reader
        .decode()
        .map_err(|e| Error::Undecodable { path: path.to_path_buf(), reason: e.to_string() })
}

/// Carry out the EXIF transform the header said to.
///
/// A total match onto the decoder's own vocabulary rather than a call to its
/// tag reader, so that [`crate::image::Orientation`] stays the single source of
/// truth for what each of the eight values means. The mirrored ones are here
/// too: they come off scanners and off every front camera, and a thumbnailer
/// that handled "the four I know about" would draw those pictures backwards.
fn turn(img: DynamicImage, orientation: Orientation) -> DynamicImage {
    use image::metadata::Orientation as Exif;

    let exif = match orientation {
        Orientation::Normal => Exif::NoTransforms,
        Orientation::FlipHorizontal => Exif::FlipHorizontal,
        Orientation::Rotate180 => Exif::Rotate180,
        Orientation::FlipVertical => Exif::FlipVertical,
        Orientation::Transpose => Exif::Rotate90FlipH,
        Orientation::Rotate90 => Exif::Rotate90,
        Orientation::Transverse => Exif::Rotate270FlipH,
        Orientation::Rotate270 => Exif::Rotate270,
    };

    let mut img = img;
    img.apply_orientation(exif);
    img
}

/// Lossy WebP, keeping alpha only when the picture has any.
///
/// The channel count is not cosmetic: an opaque photograph encoded RGBA carries
/// a whole alpha plane of 255s, and a cut-out PNG encoded RGB comes back with a
/// black rectangle where the transparency was.
fn encode(img: &DynamicImage) -> Vec<u8> {
    let (width, height) = (img.width(), img.height());
    if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        webp::Encoder::from_rgba(rgba.as_raw(), width, height).encode(QUALITY).to_vec()
    } else {
        let rgb = img.to_rgb8();
        webp::Encoder::from_rgb(rgb.as_raw(), width, height).encode(QUALITY).to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real PNG — encoded rather than hand-assembled, because unlike every
    /// other test fixture in this crate these bytes have to survive a decoder.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 0x40])
        });
        let mut out = Vec::new();
        DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn shape(webp: &[u8]) -> (u32, u32) {
        let info = crate::image::probe(webp).expect("a thumbnail has to be a readable WebP");
        (info.width, info.height)
    }

    #[test]
    fn a_thumbnail_is_a_webp_no_longer_than_the_documented_side() {
        let rendered = render(Path::new("scan.png"), &png(2000, 1000)).unwrap();
        assert_eq!(shape(&rendered), (MAX_SIDE, MAX_SIDE / 2));
        assert_eq!(crate::image::probe(&rendered).unwrap().mime, "image/webp");
    }

    #[test]
    fn the_aspect_ratio_survives_whichever_way_round_the_picture_is() {
        // Fitting to a square box rather than to a width: a 100x4000 reference
        // squashed into 512x512 is a different picture, and the grid draws
        // tiles letterboxed for exactly this reason.
        assert_eq!(shape(&render(Path::new("t.png"), &png(4000, 1000)).unwrap()), (512, 128));
        assert_eq!(shape(&render(Path::new("t.png"), &png(1000, 4000)).unwrap()), (128, 512));
    }

    #[test]
    fn a_picture_already_smaller_than_the_box_is_not_blown_up() {
        // An upscale is a bigger file conveying no more of the image.
        assert_eq!(shape(&render(Path::new("t.png"), &png(64, 48)).unwrap()), (64, 48));
    }

    #[test]
    fn every_orientation_this_crate_knows_maps_onto_one_the_decoder_knows() {
        // The mapping is total and tag-for-tag. A wrong arm here draws a
        // photograph backwards, which is the kind of bug that survives review
        // because the picture still looks like a picture.
        for tag in 1..=8u32 {
            let ours = Orientation::from_tag(tag).unwrap();
            let img = DynamicImage::ImageRgb8(image::RgbImage::new(4, 2));
            let turned = turn(img, ours);
            // The four transposing values swap the sides and the other four
            // leave them alone — the same claim `Orientation::transposes`
            // makes, checked against what the decoder actually did.
            let expected = ours.applied_to(4, 2);
            assert_eq!((turned.width(), turned.height()), expected, "tag {tag}");
        }
    }

    #[test]
    fn a_sideways_photograph_is_thumbnailed_the_way_up_it_is_meant_to_be_seen() {
        // The half of #30 the store could not finish: it records a phone
        // photograph as portrait but never rotates a pixel, because the bytes
        // on disk are the user's file. This is the one place that decodes, so
        // this is where the tag finally becomes a transform.
        let mut bytes = png(1000, 500);
        // An `eXIf` chunk saying "rotate 90° clockwise", spliced in before the
        // image data where the spec requires it.
        let exif = b"II\x2a\x00\x08\x00\x00\x00\x01\x00\x12\x01\x03\x00\x01\x00\x00\x00\x06\x00\x00\x00\x00\x00\x00\x00";
        let mut chunk = (exif.len() as u32).to_be_bytes().to_vec();
        chunk.extend_from_slice(b"eXIf");
        chunk.extend_from_slice(exif);
        chunk.extend_from_slice(&[0, 0, 0, 0]);
        let at = bytes.windows(4).position(|w| w == b"IDAT").unwrap() - 4;
        bytes.splice(at..at, chunk);

        assert_eq!(crate::image::probe(&bytes).unwrap().orientation, Orientation::Rotate90);
        // Portrait out, from a landscape file, without anything here having
        // re-read the tag.
        assert_eq!(shape(&render(Path::new("photo.png"), &bytes).unwrap()), (256, 512));
    }

    #[test]
    fn transparency_survives_and_an_opaque_picture_does_not_grow_a_channel() {
        let mut cutout = Vec::new();
        let rgba = image::RgbaImage::from_fn(200, 200, |x, _| {
            image::Rgba([0xff, 0x20, 0x40, if x < 100 { 0 } else { 255 }])
        });
        DynamicImage::ImageRgba8(rgba)
            .write_to(&mut Cursor::new(&mut cutout), image::ImageFormat::Png)
            .unwrap();

        let with_alpha = render(Path::new("cutout.png"), &cutout).unwrap();
        let decoded = image::load_from_memory(&with_alpha).unwrap();
        assert!(decoded.color().has_alpha(), "the transparent half was flattened");
        assert_eq!(decoded.to_rgba8().get_pixel(10, 10).0[3], 0);

        // And the other way: an opaque photograph must not carry a plane of
        // 255s around a share for no reason.
        let opaque = render(Path::new("photo.png"), &png(200, 200)).unwrap();
        assert!(!image::load_from_memory(&opaque).unwrap().color().has_alpha());
    }

    #[test]
    fn bytes_that_will_not_decode_are_refused_by_name_rather_than_panicking() {
        // A blob a sync client is still copying parses as a header — that is
        // how it got into the library — and stops in the middle of its pixels.
        // Saying so is what lets the caller leave the thumbnail for later
        // instead of writing a broken one.
        let truncated = &png(400, 400)[..60];
        let outcome = render(Path::new("half.png"), truncated);
        assert!(matches!(outcome, Err(Error::Undecodable { .. })), "{outcome:?}");
    }
}
