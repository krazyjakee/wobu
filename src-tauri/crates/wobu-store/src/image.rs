//! Reading an image's format, shape and frame count out of its own header.
//!
//! Wobu names an imported blob after the hash of its bytes, so the extension on
//! disk has to come from what the bytes *are* rather than from what the file
//! was called when the user dragged it in. Trusting the incoming filename would
//! put the same picture at `…c1.png` and `…c1.PNG` depending on who imported
//! it, and the content-addressed folder stops being conflict-free the moment
//! two people can produce two paths for one hash.
//!
//! The four formats below are the ones a reference image actually arrives as.
//! Everything else — a PDF, a PSD, a text file a sync client mangled — comes
//! back as `None`, which the caller turns into a refused import rather than an
//! asset nothing can display.
//!
//! Parsed by hand, and deliberately: every read is bounds-checked and returns
//! `None` on a short buffer, because half a file is the normal state of a file
//! on a share and a decoder that panics on one takes the whole app with it.
//!
//! # TIFF, single-page or multi-page, is refused (#30)
//!
//! There is no TIFF parser here and there is not going to be one. A `.tif` is
//! a container of *pages*, and the only honest answers to "which page is the
//! reference image" are "ask" and "refuse" — and asking is a modal dialog in
//! front of a drag-and-drop, for a format no provider in
//! `docs/08-providers.md` accepts anyway. So a TIFF, however many pages it
//! holds, falls out of [`probe`] as `None` and is refused by name as
//! [`Error::NotAnImage`], whose message lists the formats that do work. That
//! is the whole of the "don't half-support it" answer for TIFF: one rule, no
//! page count involved.
//!
//! # Metadata: preserved, because the bytes are never rewritten (#30)
//!
//! An import writes the user's file through unchanged, so every EXIF block in
//! it survives — including the GPS tag and the camera serial number, in a
//! folder `docs/07-file-shares.md` expects to be put on a shared drive. That
//! is a deliberate trade rather than an oversight, and the alternative was
//! considered and rejected:
//!
//! - **The path is a pure function of the bytes.** Stripping metadata, or
//!   baking the orientation into the pixels, makes the hash a function of our
//!   encoder as well as of the picture — so two collaborators on two builds
//!   would produce two files and two asset ids for one photograph, and
//!   `assets`' conflict-free claim would be gone.
//! - **A re-encode of a JPEG is lossy.** The original is the archival copy in
//!   an app whose premise is that the folder is canonical; quietly degrading it
//!   on the way in is not a privacy feature.
//!
//! So stripping belongs on the way *out* — in whatever builds a provider
//! payload — where the bytes are a copy that nothing addresses by hash. Not
//! here.
//!
//! **Orientation is the exception, and it is applied rather than preserved.**
//! [`ImageInfo::width`] and [`ImageInfo::height`] are the dimensions as
//! *displayed*: a portrait photograph out of a phone, which stores its pixels
//! landscape and a "rotate 90°" tag beside them, reads as portrait here and is
//! recorded as portrait on the `Asset`. Ignoring the tag would put every phone
//! photo in the library on its side and would measure the wrong side against
//! the 3D bounds in [`crate::assets`]. The transform itself rides along as
//! [`ImageInfo::orientation`] so that whatever decodes the pixels — a
//! thumbnailer, a provider payload — applies exactly this and does not
//! re-derive its own answer from the same tag.

use std::fs;
use std::path::Path;

use crate::error::{Error, Result};

/// What the header says. `ext` is the only spelling Wobu will ever write for
/// this format, so that one hash means one path everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    pub mime: &'static str,
    pub ext: &'static str,
    /// Width **as displayed** — EXIF orientation already applied. See the
    /// module docs.
    pub width: u32,
    /// Height as displayed, on the same terms as [`width`](Self::width).
    pub height: u32,
    /// How the stored pixels have to be turned to become the displayed image.
    /// [`Orientation::Normal`] for everything that does not say otherwise.
    pub orientation: Orientation,
    /// Whether this file holds one frame or several.
    pub frames: Frames,
}

impl ImageInfo {
    /// The dimensions the file states before orientation is applied — what a
    /// decoder will hand back, and what has to be turned by
    /// [`orientation`](Self::orientation) to match [`width`](Self::width).
    pub fn stored_size(&self) -> (u32, u32) {
        // The transform is its own inverse on the shape, so this is the same
        // swap in the other direction.
        self.orientation.applied_to(self.width, self.height)
    }

    fn with_orientation(mut self, orientation: Orientation) -> ImageInfo {
        let (width, height) = orientation.applied_to(self.width, self.height);
        self.orientation = orientation;
        self.width = width;
        self.height = height;
        self
    }
}

/// How many frames a file holds, which decides whether Wobu will store it.
///
/// A reference image is one picture. An animation is a *sequence*, and there
/// is no frame of it that Wobu could pick without either guessing on the
/// user's behalf or re-encoding — and re-encoding is what the module docs rule
/// out. So [`Multiple`](Self::Multiple) is refused at import by name, rather
/// than accepted and then silently treated as its first frame by every stage
/// downstream. That is the "pick a frame or refuse" of #30, answered with
/// refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Frames {
    #[default]
    Single,
    Multiple,
    /// The buffer ended before the question was settled.
    ///
    /// Only ever a *truncated* file: a prefix read that stopped mid-walk, or a
    /// blob a sync client is still copying. It is deliberately not the same
    /// answer as [`Multiple`](Self::Multiple) — half a file is the ordinary
    /// state of a file on a share, and refusing every one of them would refuse
    /// pictures that are perfectly fine a second later. [`probe_file`] resolves
    /// it by reading the rest of the file; an import already holds every byte,
    /// so an `Unknown` there means the file itself stops early, which is a
    /// different complaint with a different answer.
    Unknown,
}

/// The EXIF orientation tag, as the transform it describes.
///
/// Eight values, and all eight are here rather than only the four rotations:
/// the mirrored ones come off scanners and off any phone with a front camera,
/// and a decoder handed "one of the four I know about" for one of them draws
/// the picture backwards. The enum is the single source of truth for the
/// transform so that the store, the thumbnailer and a provider payload cannot
/// each read the tag and disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    /// Tag 1, and the answer for every file with no tag at all.
    #[default]
    Normal,
    /// Tag 2.
    FlipHorizontal,
    /// Tag 3.
    Rotate180,
    /// Tag 4.
    FlipVertical,
    /// Tag 5 — mirrored, then rotated 270° clockwise.
    Transpose,
    /// Tag 6 — rotated 90° clockwise for display.
    Rotate90,
    /// Tag 7 — mirrored, then rotated 90° clockwise.
    Transverse,
    /// Tag 8 — rotated 270° clockwise for display.
    Rotate270,
}

impl Orientation {
    /// The tag value, or `None` for the values EXIF does not define — which is
    /// read as "no usable orientation" rather than guessed at, because a
    /// corrupt tag turning a landscape reference on its side is worse than an
    /// un-rotated one.
    pub fn from_tag(tag: u32) -> Option<Orientation> {
        Some(match tag {
            1 => Orientation::Normal,
            2 => Orientation::FlipHorizontal,
            3 => Orientation::Rotate180,
            4 => Orientation::FlipVertical,
            5 => Orientation::Transpose,
            6 => Orientation::Rotate90,
            7 => Orientation::Transverse,
            8 => Orientation::Rotate270,
            _ => return None,
        })
    }

    /// Whether this transform swaps the two sides.
    pub fn transposes(self) -> bool {
        matches!(
            self,
            Orientation::Transpose
                | Orientation::Rotate90
                | Orientation::Transverse
                | Orientation::Rotate270
        )
    }

    /// `(width, height)` after the transform. The mirrors and the 180° turn
    /// leave the shape alone; the four transposing ones swap it.
    pub fn applied_to(self, width: u32, height: u32) -> (u32, u32) {
        if self.transposes() { (height, width) } else { (width, height) }
    }
}

/// How much of a file to read before giving up and reading all of it.
///
/// PNG, GIF and WebP declare their size in the first few dozen bytes. JPEG
/// does not: the SOF segment sits after every APP segment, and an EXIF block
/// carrying an embedded preview is routinely tens of kilobytes. This is the
/// size at which "read the header" stops being cheaper than "read the file"
/// over SMB anyway.
const PROBE_BYTES: usize = 64 * 1024;

/// Identify a blob we already hold in memory.
pub fn probe(bytes: &[u8]) -> Option<ImageInfo> {
    png(bytes).or_else(|| gif(bytes)).or_else(|| webp(bytes)).or_else(|| jpeg(bytes))
}

/// Identify a file on disk without necessarily reading all of it.
///
/// The whole file is read only when the prefix ran out mid-parse, which on a
/// rebuild of a large library is the difference between listing the folder and
/// pulling every megabyte of it back over the network.
///
/// A frame count that came back [`Frames::Unknown`] is the second reason to
/// pay for the rest of the file, and it is a narrow one: an animated GIF
/// declares its second frame wherever it likes, so a prefix that stops before
/// it would report an animation as a still and the import rule in
/// [`crate::assets`] would let it through. PNG and WebP settle the question in
/// their first few chunks, so this only ever costs on a GIF or on a file that
/// genuinely stops early.
pub fn probe_file(path: &Path) -> Result<Option<ImageInfo>> {
    let head = read_prefix(path, PROBE_BYTES)?;
    let truncated = head.len() == PROBE_BYTES;
    match probe(&head) {
        Some(info) if info.frames != Frames::Unknown || !truncated => return Ok(Some(info)),
        // Either nothing parsed, or the frame walk ran off the end of the
        // prefix. Both are answered by the rest of the file.
        Some(_) => {}
        None if !truncated => {
            // We had the whole file and still could not read it.
            return Ok(None);
        }
        None => {}
    }
    let all = fs::read(path).map_err(|e| Error::io(path, e))?;
    Ok(probe(&all))
}

fn read_prefix(path: &Path, limit: usize) -> Result<Vec<u8>> {
    use std::io::Read as _;
    let file = fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut buf = Vec::with_capacity(limit.min(8 * 1024));
    file.take(limit as u64).read_to_end(&mut buf).map_err(|e| Error::io(path, e))?;
    Ok(buf)
}

/* ── the four formats ─────────────────────────────────────────────────────── */

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn be_u16(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_be_bytes([slice[0], slice[1]]) as u32)
}

fn le_u16(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]) as u32)
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// The 24-bit little-endian counts WebP's extended header uses.
fn le_u24(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 3)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], 0]))
}

/// A header claiming a zero dimension is malformed, not a 0-pixel image, and
/// letting one through would divide by zero in every thumbnail that follows.
fn sized(mime: &'static str, ext: &'static str, width: u32, height: u32) -> Option<ImageInfo> {
    if width == 0 || height == 0 {
        return None;
    }
    Some(ImageInfo {
        mime,
        ext,
        width,
        height,
        orientation: Orientation::Normal,
        frames: Frames::Single,
    })
}

const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// The IHDR chunk is required by the spec to be the first one, so its size and
/// position are fixed; the walk after it is for the two chunks that change what
/// the file *is* — `acTL`, which makes it an APNG, and `eXIf`.
fn png(bytes: &[u8]) -> Option<ImageInfo> {
    if !bytes.starts_with(PNG_MAGIC) || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let info = sized("image/png", "png", be_u32(bytes, 16)?, be_u32(bytes, 20)?)?;
    Some(png_chunks(bytes, info))
}

/// Walk PNG's chunk list far enough to know whether this is an animation and
/// which way up it goes.
///
/// Stops at `IDAT`, which is where both answers are already decided: `acTL` is
/// required to precede the image data, and `eXIf` is recommended to. Walking
/// the pixel chunks of a 40 MB PNG to find neither is a lot of network for
/// nothing.
fn png_chunks(bytes: &[u8], mut info: ImageInfo) -> ImageInfo {
    // Past the signature; IHDR is the first chunk and is walked over like any
    // other, since its contents are already read above.
    let mut at = PNG_MAGIC.len();
    loop {
        let (Some(length), Some(kind)) = (be_u32(bytes, at), bytes.get(at + 4..at + 8)) else {
            // Off the end before either chunk turned up. On the whole file that
            // is a truncated PNG; on a prefix read it is a file whose ancillary
            // chunks are enormous. Either way the frame count is not settled.
            info.frames = Frames::Unknown;
            return info;
        };
        match kind {
            // An animation control chunk. There is no reading past this: the
            // file is a sequence, and `assets::import` refuses it.
            b"acTL" => {
                info.frames = Frames::Multiple;
                return info;
            }
            b"eXIf" => {
                let block = bytes.get(at + 8..at + 8 + length as usize);
                if let Some(orientation) = block.and_then(exif_orientation) {
                    info = info.with_orientation(orientation);
                }
            }
            b"IDAT" | b"IEND" => return info,
            _ => {}
        }
        // Length, type, payload, CRC. Checked because a corrupt length field
        // would otherwise wrap the cursor back to the top of the file.
        let Some(next) = at.checked_add(12).and_then(|a| a.checked_add(length as usize)) else {
            info.frames = Frames::Unknown;
            return info;
        };
        at = next;
    }
}

/// GIF's logical screen descriptor follows the six-byte signature.
fn gif(bytes: &[u8]) -> Option<ImageInfo> {
    let magic = bytes.get(..6)?;
    if magic != b"GIF87a" && magic != b"GIF89a" {
        return None;
    }
    let mut info = sized("image/gif", "gif", le_u16(bytes, 6)?, le_u16(bytes, 8)?)?;
    // GIF carries no EXIF, so there is no orientation to apply here.
    info.frames = gif_frames(bytes);
    Some(info)
}

/// Count GIF's image descriptors, stopping at the second one.
///
/// There is no header field for this — an animated GIF is simply one with more
/// than one frame block, and the `NETSCAPE2.0` loop extension that usually
/// accompanies one is a convention rather than a requirement. So the block
/// structure is walked, which is cheap (every block is length-prefixed) and,
/// unlike a heuristic, gives the same answer for a two-frame GIF that no
/// encoder bothered to mark as a loop.
fn gif_frames(bytes: &[u8]) -> Frames {
    let Some(packed) = bytes.get(10).copied() else {
        return Frames::Unknown;
    };
    // The global colour table, if the flag says there is one, sits between the
    // screen descriptor and the first block.
    let mut at = 13 + if packed & 0x80 != 0 { 3usize << ((packed & 7) + 1) } else { 0 };

    let mut descriptors = 0;
    loop {
        let Some(block) = bytes.get(at).copied() else {
            return Frames::Unknown;
        };
        at += 1;
        match block {
            // Trailer: the file is over, and we counted what we counted.
            0x3b => return Frames::Single,
            // An extension block — graphic control, comment, application. The
            // label byte, then length-prefixed sub-blocks.
            0x21 => {
                let Some(next) = gif_skip_sub_blocks(bytes, at + 1) else {
                    return Frames::Unknown;
                };
                at = next;
            }
            // An image descriptor: this is a frame.
            0x2c => {
                descriptors += 1;
                if descriptors > 1 {
                    return Frames::Multiple;
                }
                let Some(local) = bytes.get(at + 8).copied() else {
                    return Frames::Unknown;
                };
                // Nine bytes of descriptor, an optional local colour table,
                // then the LZW minimum code size, then the pixel sub-blocks.
                at += 9 + if local & 0x80 != 0 { 3usize << ((local & 7) + 1) } else { 0 } + 1;
                let Some(next) = gif_skip_sub_blocks(bytes, at) else {
                    return Frames::Unknown;
                };
                at = next;
            }
            // A byte that introduces no block GIF defines. The header parsed,
            // so this is a real GIF that has gone wrong further in, and the
            // frame count is not something we can claim to know.
            _ => return Frames::Unknown,
        }
    }
}

/// Skip a run of GIF sub-blocks, returning the offset just past the terminator.
fn gif_skip_sub_blocks(bytes: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let length = *bytes.get(at)? as usize;
        at += 1;
        if length == 0 {
            return Some(at);
        }
        at += length;
    }
}

/// WebP is a RIFF container with three incompatible ways of stating a size:
/// lossy (`VP8 `), lossless (`VP8L`) and extended (`VP8X`). A file saved from a
/// browser can be any of them, so all three are read.
fn webp(bytes: &[u8]) -> Option<ImageInfo> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    // Only the extended form can be an animation or carry EXIF, and it says so
    // in a flags byte at a fixed offset — no walk needed to settle the frame
    // count, unlike GIF.
    let mut frames = Frames::Single;
    // Chunk header at 12 (fourcc + length), so every payload starts at 20.
    let (width, height) = match bytes.get(12..16)? {
        b"VP8 " => {
            // The three-byte frame tag is followed by a sync code that is the
            // only thing distinguishing a keyframe from a fragment we cannot
            // read a size from.
            if bytes.get(23..26)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            (le_u16(bytes, 26)? & 0x3fff, le_u16(bytes, 28)? & 0x3fff)
        }
        b"VP8L" => {
            if *bytes.get(20)? != 0x2f {
                return None;
            }
            let slice = bytes.get(21..25)?;
            let packed = u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]);
            ((packed & 0x3fff) + 1, ((packed >> 14) & 0x3fff) + 1)
        }
        // Flags and reserved bits occupy the first four bytes of the payload;
        // the canvas size is stored minus one, so a 1x1 image reads as zero.
        b"VP8X" => {
            // Bit 1 of the flags byte is ANIM. A file with it set has its
            // frames in `ANMF` chunks and no still image at all, so treating
            // it as one would be reading a canvas size and nothing else.
            if bytes.get(20)? & 0x02 != 0 {
                frames = Frames::Multiple;
            }
            (le_u24(bytes, 24)? + 1, le_u24(bytes, 27)? + 1)
        }
        _ => return None,
    };
    let mut info = sized("image/webp", "webp", width, height)?;
    info.frames = frames;
    if let Some(orientation) = riff_exif(bytes) {
        info = info.with_orientation(orientation);
    }
    Some(info)
}

/// The orientation in a WebP's `EXIF` chunk, if it has one.
///
/// Walking the chunk list rather than reading a fixed offset: `EXIF` is
/// optional, appears after the image data, and its position depends on which
/// of `ICCP`, `ALPH` and `ANIM` the encoder wrote first.
fn riff_exif(bytes: &[u8]) -> Option<Orientation> {
    let mut at = 12;
    loop {
        let kind = bytes.get(at..at + 4)?;
        let length = le_u32(bytes, at + 4)? as usize;
        if kind == b"EXIF" {
            return bytes.get(at + 8..at + 8 + length).and_then(exif_orientation);
        }
        // RIFF pads odd-length payloads to an even boundary.
        at = at.checked_add(8)?.checked_add(length + (length & 1))?;
    }
}

/// Whether a marker introduces a start-of-frame segment, which is the only
/// place a JPEG states its size.
///
/// The exclusions matter: `C4`, `C8` and `CC` sit inside the same numeric range
/// and are a Huffman table, a reserved extension and an arithmetic coding table.
/// Reading any of them as a frame header yields a plausible-looking and
/// completely wrong pair of numbers.
fn is_sof(marker: u8) -> bool {
    matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc)
}

/// Walk the marker segments to the first SOF, reading the EXIF orientation on
/// the way past it.
///
/// There is no fixed offset to read here: APP0/APP1/APP2 blocks of arbitrary
/// size come first, and a photograph out of a camera carries an EXIF preview
/// large enough that the frame header is many kilobytes in. That walk is
/// already paid for, and the APP1 it walks over is exactly where the
/// orientation lives, so reading it costs nothing extra.
///
/// A JPEG is one frame. Multi-picture JPEGs (an MPO out of a 3D camera) exist,
/// declare their extra images in an APP2 this walk skips, and are read as their
/// first image by every decoder including this one — which is the same answer
/// everything else in the pipeline will give, so it is not a half-supported
/// case in the sense #30 is about.
fn jpeg(bytes: &[u8]) -> Option<ImageInfo> {
    if bytes.get(..2)? != [0xff, 0xd8] {
        return None;
    }

    let mut orientation = Orientation::Normal;
    let mut at = 2;
    loop {
        // Any number of 0xff fill bytes may pad the gap before a marker.
        while *bytes.get(at)? == 0xff && *bytes.get(at + 1)? == 0xff {
            at += 1;
        }
        if *bytes.get(at)? != 0xff {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        at += 2;

        match marker {
            // Standalone markers: no length field follows.
            0x01 | 0xd0..=0xd8 => continue,
            // End of image, and start of scan — the entropy-coded data after
            // SOS is not marker-structured, and a valid JPEG has already given
            // us its SOF by then. Walking past either is reading noise.
            0xd9 | 0xda => return None,
            _ => {}
        }

        let length = be_u16(bytes, at)? as usize;
        // A segment shorter than its own length field is a corrupt file, and
        // advancing by it would loop forever.
        if length < 2 {
            return None;
        }
        if is_sof(marker) {
            // Sample precision, then height, then width.
            let info = sized("image/jpeg", "jpg", be_u16(bytes, at + 5)?, be_u16(bytes, at + 3)?)?;
            return Some(info.with_orientation(orientation));
        }
        // APP1 is where EXIF lives, and only the first one counts: a second
        // APP1 is XMP or a duplicate, and the first is what a decoder honours.
        if marker == 0xe1
            && orientation == Orientation::Normal
            && let Some(found) = bytes.get(at + 2..at + length).and_then(exif_orientation)
        {
            orientation = found;
        }
        at += length;
    }
}

/* ── EXIF ─────────────────────────────────────────────────────────────────── */

/// The orientation tag out of an EXIF block, wherever the block came from.
///
/// One reader for all three carriers — JPEG's APP1, PNG's `eXIf`, WebP's
/// `EXIF` — because they hold the identical TIFF structure and two copies of
/// this would be two chances to disagree about which way a photograph goes.
/// JPEG prefixes the block with `Exif\0\0`; the chunk forms do not, and some
/// encoders write it anyway, so both are accepted.
///
/// Only IFD0 is read. Orientation is defined to live there, and following the
/// Exif sub-IFD pointer to look for a second copy would be walking a
/// user-supplied offset graph for a tag that is not allowed to be in it.
fn exif_orientation(block: &[u8]) -> Option<Orientation> {
    const ORIENTATION_TAG: u32 = 0x0112;

    let tiff = match block.get(..6) {
        Some(b"Exif\0\0") => block.get(6..)?,
        _ => block,
    };
    // `II` little-endian, `MM` big-endian — the one field in EXIF that is not
    // itself byte-ordered, and getting it wrong reads tag 0x0112 as 0x1201.
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let short = |at: usize| if little { le_u16(tiff, at) } else { be_u16(tiff, at) };
    let long = |at: usize| if little { le_u32(tiff, at) } else { be_u32(tiff, at) };

    if short(2)? != 42 {
        return None;
    }
    let ifd = long(4)? as usize;
    let entries = short(ifd)? as usize;
    for i in 0..entries {
        // Twelve bytes an entry, after the two-byte count: tag, type, count,
        // then a value field that holds the value itself when it fits.
        let entry = ifd.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
        if short(entry)? == ORIENTATION_TAG {
            // A SHORT, so it sits in the first two bytes of the value field
            // regardless of which end of it the remaining two are.
            return Orientation::from_tag(short(entry + 8)?);
        }
    }
    None
}
#[cfg(test)]
mod tests;
