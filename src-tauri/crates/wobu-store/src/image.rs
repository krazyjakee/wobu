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
mod tests {
    use super::*;

    /// A 2x3 PNG, header only — everything after IHDR is irrelevant here.
    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut out = PNG_MAGIC.to_vec();
        out.extend_from_slice(&13u32.to_be_bytes());
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0]);
        out
    }

    fn gif_bytes(width: u16, height: u16) -> Vec<u8> {
        let mut out = b"GIF89a".to_vec();
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&[0xf7, 0x00, 0x00]);
        out
    }

    /// A minimal JPEG: SOI, an APP0 block to walk past, then SOF0.
    fn jpeg_bytes(width: u16, height: u16) -> Vec<u8> {
        let mut out = vec![0xff, 0xd8];
        out.extend_from_slice(&[0xff, 0xe0, 0x00, 0x10]);
        out.extend_from_slice(b"JFIF\0");
        out.extend_from_slice(&[0; 9]);
        out.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        out
    }

    fn riff(chunk: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&((payload.len() + 12) as u32).to_le_bytes());
        out.extend_from_slice(b"WEBP");
        out.extend_from_slice(chunk);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn webp_lossy(width: u16, height: u16) -> Vec<u8> {
        let mut payload = vec![0x00, 0x00, 0x00, 0x9d, 0x01, 0x2a];
        payload.extend_from_slice(&width.to_le_bytes());
        payload.extend_from_slice(&height.to_le_bytes());
        riff(b"VP8 ", &payload)
    }

    fn webp_lossless(width: u32, height: u32) -> Vec<u8> {
        let packed = (width - 1) | ((height - 1) << 14);
        let mut payload = vec![0x2f];
        payload.extend_from_slice(&packed.to_le_bytes());
        riff(b"VP8L", &payload)
    }

    fn webp_extended(width: u32, height: u32) -> Vec<u8> {
        webp_vp8x(width, height, 0x10, &[])
    }

    /// An extended WebP with the flags byte spelled out, and any trailing
    /// chunks appended after the `VP8X` one. `0x10` is the alpha flag every
    /// other test here uses; `0x02` is ANIM and `0x08` is EXIF.
    fn webp_vp8x(width: u32, height: u32, flags: u8, tail: &[u8]) -> Vec<u8> {
        let mut payload = vec![flags, 0, 0, 0];
        payload.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        payload.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        let mut out = riff(b"VP8X", &payload);
        out.extend_from_slice(tail);
        // The RIFF length covers everything after it, so appending has to fix
        // it up or the chunk walk stops at the wrong place.
        let size = (out.len() - 8) as u32;
        out[4..8].copy_from_slice(&size.to_le_bytes());
        out
    }

    fn riff_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = kind.to_vec();
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        // RIFF pads an odd payload to an even boundary.
        if payload.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    /// Signature and logical screen descriptor, with the global-colour-table
    /// flag clear so the first block follows immediately — which is what keeps
    /// these fixtures short enough to write out by hand.
    fn gif_header(width: u16, height: u16) -> Vec<u8> {
        let mut out = b"GIF89a".to_vec();
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&[0x00, 0x00, 0x00]);
        out
    }

    /// One frame: a graphic control extension, as any animator writes, then an
    /// image descriptor and `data` bytes of pixel sub-blocks.
    fn gif_frame(width: u16, height: u16, data: usize) -> Vec<u8> {
        let mut out = vec![0x21, 0xf9, 0x04, 0x00, 0x0a, 0x00, 0x00, 0x00];
        out.extend_from_slice(&[0x2c, 0, 0, 0, 0]);
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        // No local colour table, then the LZW minimum code size.
        out.extend_from_slice(&[0x00, 0x02]);
        let mut left = data;
        while left > 0 {
            // A sub-block's length field is one byte, so a large frame is
            // spread over many of them — exactly as a real one is.
            let run = left.min(255);
            out.push(run as u8);
            out.extend_from_slice(&vec![0u8; run]);
            left -= run;
        }
        out.push(0x00);
        out
    }

    /// A complete GIF with `frames` image descriptors in it.
    fn gif_animation(width: u16, height: u16, frames: usize) -> Vec<u8> {
        let mut out = gif_header(width, height);
        for _ in 0..frames {
            out.extend_from_slice(&gif_frame(width, height, 0));
        }
        out.push(0x3b);
        out
    }

    fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = (payload.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        // The CRC is never checked here — this parser reads structure, not
        // integrity, and a decoder further on is what validates the pixels.
        out.extend_from_slice(&[0, 0, 0, 0]);
        out
    }

    /// A whole PNG rather than a bare header: signature, IHDR, whatever
    /// ancillary chunks the test is about, then IDAT and IEND.
    fn png_file(width: u32, height: u32, ancillary: &[Vec<u8>]) -> Vec<u8> {
        let mut ihdr = width.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);

        let mut out = PNG_MAGIC.to_vec();
        out.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
        for chunk in ancillary {
            out.extend_from_slice(chunk);
        }
        out.extend_from_slice(&png_chunk(b"IDAT", &[0; 8]));
        out.extend_from_slice(&png_chunk(b"IEND", &[]));
        out
    }

    /// An EXIF block carrying one orientation tag and nothing else.
    ///
    /// `marker` writes the `Exif\0\0` prefix a JPEG's APP1 carries and a PNG's
    /// `eXIf` chunk does not; `little` picks the byte order, which is the field
    /// most worth having both ways in a test.
    fn exif(tag: u16, marker: bool, little: bool) -> Vec<u8> {
        let mut out = if marker { b"Exif\0\0".to_vec() } else { Vec::new() };
        let short = |v: u16| if little { v.to_le_bytes() } else { v.to_be_bytes() };
        let long = |v: u32| if little { v.to_le_bytes() } else { v.to_be_bytes() };

        out.extend_from_slice(if little { b"II" } else { b"MM" });
        out.extend_from_slice(&short(42));
        // IFD0 sits directly after the eight-byte TIFF header.
        out.extend_from_slice(&long(8));
        out.extend_from_slice(&short(1));
        out.extend_from_slice(&short(0x0112));
        // Type SHORT, one value, in the first half of the value field.
        out.extend_from_slice(&short(3));
        out.extend_from_slice(&long(1));
        out.extend_from_slice(&short(tag));
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&long(0));
        out
    }

    /// A JPEG whose APP1 holds `block`, walked over on the way to SOF0.
    fn jpeg_with_app1(width: u16, height: u16, block: &[u8]) -> Vec<u8> {
        let mut out = vec![0xff, 0xd8, 0xff, 0xe1];
        out.extend_from_slice(&((block.len() + 2) as u16).to_be_bytes());
        out.extend_from_slice(block);
        out.extend_from_slice(&jpeg_bytes(width, height)[2..]);
        out
    }

    #[test]
    fn every_supported_format_reports_its_own_dimensions() {
        let cases: [(Vec<u8>, &str, &str); 6] = [
            (png_bytes(640, 480), "image/png", "png"),
            (gif_bytes(640, 480), "image/gif", "gif"),
            (jpeg_bytes(640, 480), "image/jpeg", "jpg"),
            (webp_lossy(640, 480), "image/webp", "webp"),
            (webp_lossless(640, 480), "image/webp", "webp"),
            (webp_extended(640, 480), "image/webp", "webp"),
        ];
        for (bytes, mime, ext) in cases {
            let info = probe(&bytes).unwrap_or_else(|| panic!("{mime} did not parse"));
            assert_eq!((info.width, info.height), (640, 480), "{mime}");
            assert_eq!(info.mime, mime);
            assert_eq!(info.ext, ext);
        }
    }

    #[test]
    fn a_jpeg_states_its_height_before_its_width() {
        // The SOF segment is precision, height, width — the opposite order to
        // every other format here, and reading it the intuitive way swaps a
        // portrait reference into a landscape one silently.
        let info = probe(&jpeg_bytes(1024, 768)).unwrap();
        assert_eq!((info.width, info.height), (1024, 768));
    }

    #[test]
    fn a_huffman_table_is_not_mistaken_for_a_frame_header() {
        // DHT (0xc4) sits in the middle of the SOF marker range. Treating it as
        // a frame header reads table data as a size and returns nonsense that
        // looks entirely plausible.
        // SOI, then a DHT whose length field covers itself plus three bytes.
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xc4, 0x00, 0x05, 0x00, 0x11, 0x22];
        bytes.extend_from_slice(&jpeg_bytes(320, 200)[2..]);
        let info = probe(&bytes).unwrap();
        assert_eq!((info.width, info.height), (320, 200));
    }

    #[test]
    fn a_truncated_file_is_unreadable_rather_than_a_panic() {
        // Half a file is the ordinary state of a file a sync client is still
        // copying, and every one of these used to index out of bounds.
        for full in [png_bytes(64, 64), gif_bytes(64, 64), jpeg_bytes(64, 64), webp_lossy(64, 64)] {
            for cut in 0..full.len() {
                probe(&full[..cut]);
            }
        }
    }

    #[test]
    fn bytes_that_are_not_an_image_are_refused_rather_than_guessed_at() {
        for bytes in [
            &b""[..],
            b"not an image at all",
            b"%PDF-1.7\n",
            // A RIFF container that is not WebP, and a WebP with a codec chunk
            // nothing here can read: both must fail rather than return zeroes.
            b"RIFF\x24\x00\x00\x00WAVEfmt ",
            &riff(b"ANIM", &[0; 32])[..],
        ] {
            assert!(probe(bytes).is_none(), "{bytes:?} was accepted");
        }
    }

    #[test]
    fn a_header_claiming_a_zero_dimension_is_refused() {
        // Nothing downstream divides by zero if this never becomes an asset.
        assert!(probe(&png_bytes(0, 480)).is_none());
        assert!(probe(&gif_bytes(640, 0)).is_none());
    }

    #[test]
    fn a_jpeg_whose_frame_header_is_past_the_probe_window_is_still_read() {
        // An EXIF preview pushes SOF tens of kilobytes in, so a prefix read
        // alone would report a photograph out of a camera as "not an image".
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");

        // A marker segment's length field is 16 bits, so a real camera file
        // spreads a large preview across several APP segments — as this does.
        let mut bytes = vec![0xff, 0xd8];
        while bytes.len() < PROBE_BYTES + 1024 {
            let payload = 0xfffcusize;
            bytes.extend_from_slice(&[0xff, 0xe1]);
            bytes.extend_from_slice(&((payload + 2) as u16).to_be_bytes());
            bytes.extend_from_slice(&vec![0u8; payload]);
        }
        bytes.extend_from_slice(&jpeg_bytes(4032, 3024)[2..]);
        std::fs::write(&path, &bytes).unwrap();

        let info = probe_file(&path).unwrap().unwrap();
        assert_eq!((info.width, info.height), (4032, 3024));
    }

    /* ── frames (#30) ─────────────────────────────────────────────────── */

    #[test]
    fn a_still_and_an_animation_of_the_same_size_are_told_apart() {
        // Nothing in a GIF's header says "animated": the answer is the number
        // of image descriptors in the block list, so both of these have byte
        // -for-byte identical first thirteen bytes.
        assert_eq!(probe(&gif_animation(64, 64, 1)).unwrap().frames, Frames::Single);
        assert_eq!(probe(&gif_animation(64, 64, 2)).unwrap().frames, Frames::Multiple);
        assert_eq!(probe(&gif_animation(64, 64, 30)).unwrap().frames, Frames::Multiple);
    }

    #[test]
    fn a_gif_that_never_declares_a_loop_is_still_counted_as_an_animation() {
        // The usual shortcut is to look for the `NETSCAPE2.0` application
        // extension, which is a convention rather than a requirement — a
        // two-frame GIF without one plays perfectly well and would sail past
        // that check as a still.
        let bytes = gif_animation(48, 48, 2);
        let looping = b"NETSCAPE2.0";
        assert!(
            !bytes.windows(looping.len()).any(|w| w == looping),
            "the fixture is not the case this test is about",
        );
        assert_eq!(probe(&bytes).unwrap().frames, Frames::Multiple);
    }

    #[test]
    fn an_apng_is_an_animation_and_an_ordinary_png_is_not() {
        // `acTL` is the whole difference, and it is required to precede the
        // image data — which is why the chunk walk can stop at `IDAT`.
        let still = png_file(64, 64, &[]);
        let apng = png_file(64, 64, &[png_chunk(b"acTL", &[0, 0, 0, 4, 0, 0, 0, 0])]);

        assert_eq!(probe(&still).unwrap().frames, Frames::Single);
        assert_eq!(probe(&apng).unwrap().frames, Frames::Multiple);
        // And an APNG still reports its size, so the refusal downstream can be
        // about the frames rather than about the file being unreadable.
        assert_eq!((probe(&apng).unwrap().width, probe(&apng).unwrap().height), (64, 64));
    }

    #[test]
    fn an_animated_webp_is_read_off_its_flags_byte() {
        assert_eq!(probe(&webp_vp8x(200, 100, 0x02, &[])).unwrap().frames, Frames::Multiple);
        assert_eq!(probe(&webp_vp8x(200, 100, 0x10, &[])).unwrap().frames, Frames::Single);
        // Lossy and lossless WebP have no way to hold more than one frame.
        assert_eq!(probe(&webp_lossy(200, 100)).unwrap().frames, Frames::Single);
        assert_eq!(probe(&webp_lossless(200, 100)).unwrap().frames, Frames::Single);
    }

    #[test]
    fn a_gif_whose_blocks_run_off_the_end_says_it_does_not_know() {
        // The distinction the import rule rests on. A file that stops early is
        // a file a sync client is still copying — not an animation — and
        // answering `Multiple` here would refuse pictures that are fine a
        // second later.
        let full = gif_animation(64, 64, 2);
        for cut in 13..full.len() - 1 {
            let frames = probe(&full[..cut]).map(|i| i.frames);
            assert_ne!(frames, Some(Frames::Single), "cut {cut} claimed a still");
        }
        assert_eq!(probe(&full[..20]).unwrap().frames, Frames::Unknown);
    }

    #[test]
    fn an_animation_hiding_past_the_probe_window_is_found_anyway() {
        // The reason `probe_file` pays for the rest of the file on an
        // `Unknown`. A GIF's second frame can sit anywhere, so a 64 KB prefix
        // read of a large one settles nothing — and reporting it as a still
        // would let it into the library, which is the whole failure #30 names.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("loop.gif");

        // A first frame fat enough to push the second one past the window.
        let mut bytes = gif_header(320, 200);
        bytes.extend_from_slice(&gif_frame(320, 200, PROBE_BYTES + 4096));
        bytes.extend_from_slice(&gif_frame(320, 200, 0));
        bytes.push(0x3b);
        assert!(bytes.len() > PROBE_BYTES, "the fixture has to outgrow the probe window");
        std::fs::write(&path, &bytes).unwrap();

        let info = probe_file(&path).unwrap().unwrap();
        assert_eq!((info.width, info.height), (320, 200));
        assert_eq!(info.frames, Frames::Multiple, "a prefix read reported an animation as a still");
    }

    #[test]
    fn a_tiff_is_not_an_image_this_crate_reads_however_many_pages_it_has() {
        // Both byte orders, and a second IFD offset for the multi-page case —
        // which changes nothing, and that is the point: one rule, no page
        // count involved.
        let mut little = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
        little.extend_from_slice(&[0; 32]);
        let mut big = b"MM\x00\x2a\x00\x00\x00\x08".to_vec();
        big.extend_from_slice(&[0; 32]);

        assert!(probe(&little).is_none());
        assert!(probe(&big).is_none());
    }

    /* ── orientation (#30) ────────────────────────────────────────────── */

    #[test]
    fn a_photograph_tagged_sideways_reports_the_shape_it_is_meant_to_be_seen_as() {
        // A phone shoots landscape and writes "rotate 90°" beside it. Reading
        // the SOF alone puts every portrait photo in the library on its side,
        // and measures the wrong side against the 3D bounds.
        let info = probe(&jpeg_with_app1(4032, 3024, &exif(6, true, true))).unwrap();
        assert_eq!((info.width, info.height), (3024, 4032));
        assert_eq!(info.orientation, Orientation::Rotate90);
        // The file's own numbers are still reachable, because whatever decodes
        // the pixels gets those and has to turn them itself.
        assert_eq!(info.stored_size(), (4032, 3024));
    }

    #[test]
    fn the_four_transposing_orientations_swap_the_sides_and_the_other_four_do_not() {
        // Mirrors and the 180° turn leave the shape alone. Swapping on one of
        // those would report a landscape reference as portrait for no reason
        // at all, which is the same bug in the other direction.
        for (tag, expected) in [(1, (800, 600)), (2, (800, 600)), (3, (800, 600)), (4, (800, 600))]
        {
            let info = probe(&jpeg_with_app1(800, 600, &exif(tag, true, true))).unwrap();
            assert_eq!((info.width, info.height), expected, "tag {tag}");
            assert!(!info.orientation.transposes(), "tag {tag}");
        }
        for tag in [5, 6, 7, 8] {
            let info = probe(&jpeg_with_app1(800, 600, &exif(tag, true, true))).unwrap();
            assert_eq!((info.width, info.height), (600, 800), "tag {tag}");
            assert!(info.orientation.transposes(), "tag {tag}");
        }
    }

    #[test]
    fn an_orientation_reads_the_same_from_either_byte_order() {
        // The one EXIF field that is not itself byte-ordered. Get it wrong and
        // tag 0x0112 is read as 0x1201, which matches nothing and silently
        // leaves every big-endian camera's photos un-rotated.
        for little in [true, false] {
            let info = probe(&jpeg_with_app1(1000, 500, &exif(8, true, little))).unwrap();
            assert_eq!(info.orientation, Orientation::Rotate270, "little: {little}");
            assert_eq!((info.width, info.height), (500, 1000));
        }
    }

    #[test]
    fn orientation_is_read_out_of_a_png_chunk_and_a_webp_chunk_too() {
        // One EXIF reader for all three carriers. A second copy of it is a
        // second chance to disagree about which way up a picture goes.
        let png = png_file(900, 300, &[png_chunk(b"eXIf", &exif(6, false, true))]);
        assert_eq!(probe(&png).unwrap().orientation, Orientation::Rotate90);
        assert_eq!((probe(&png).unwrap().width, probe(&png).unwrap().height), (300, 900));

        let webp = webp_vp8x(900, 300, 0x08, &riff_chunk(b"EXIF", &exif(6, false, true)));
        assert_eq!(probe(&webp).unwrap().orientation, Orientation::Rotate90);
        assert_eq!((probe(&webp).unwrap().width, probe(&webp).unwrap().height), (300, 900));
    }

    #[test]
    fn a_file_with_no_orientation_or_a_nonsense_one_is_left_alone() {
        // A corrupt tag turning a landscape reference on its side is worse
        // than an un-rotated one, so anything outside 1–8 is not guessed at.
        let plain = probe(&jpeg_bytes(640, 480)).unwrap();
        assert_eq!(plain.orientation, Orientation::Normal);
        assert_eq!((plain.width, plain.height), (640, 480));

        for tag in [0, 9, 255] {
            let info = probe(&jpeg_with_app1(640, 480, &exif(tag, true, true))).unwrap();
            assert_eq!(info.orientation, Orientation::Normal, "tag {tag}");
            assert_eq!((info.width, info.height), (640, 480), "tag {tag}");
        }
        assert_eq!(Orientation::from_tag(9), None);
    }

    #[test]
    fn a_truncated_or_lying_exif_block_never_panics_and_never_rotates() {
        // The same rule as every other parser here: half a file is ordinary,
        // and this one walks an offset the file itself supplies.
        let full = exif(6, true, true);
        for cut in 0..full.len() {
            let info = probe(&jpeg_with_app1(640, 480, &full[..cut])).unwrap();
            // The SOF states the size; all a half-read EXIF block can do is
            // turn it or not, and it must never invent one or index past its
            // own end trying. (A cut that happens to land after the whole
            // orientation entry legitimately rotates — the tag was there.)
            assert!(
                (info.width, info.height) == (640, 480) || (info.width, info.height) == (480, 640),
                "cut {cut}: {info:?}",
            );
        }
        // An IFD offset pointing past the end of the block, which is what a
        // mangled file supplies and what an unchecked read would follow.
        let mut liar = exif(6, true, true);
        liar[10..14].copy_from_slice(&0xffff_fff0u32.to_le_bytes());
        assert_eq!(
            probe(&jpeg_with_app1(640, 480, &liar)).unwrap().orientation,
            Orientation::Normal,
        );
    }
}
