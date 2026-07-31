//! Reading an image's format and dimensions out of its own header.
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

use std::fs;
use std::path::Path;

use crate::error::{Error, Result};

/// What the header says. `ext` is the only spelling Wobu will ever write for
/// this format, so that one hash means one path everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    pub mime: &'static str,
    pub ext: &'static str,
    pub width: u32,
    pub height: u32,
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
pub fn probe_file(path: &Path) -> Result<Option<ImageInfo>> {
    let head = read_prefix(path, PROBE_BYTES)?;
    if let Some(info) = probe(&head) {
        return Ok(Some(info));
    }
    if head.len() < PROBE_BYTES {
        // We had the whole file and still could not read it.
        return Ok(None);
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
    Some(ImageInfo { mime, ext, width, height })
}

const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// The IHDR chunk is required by the spec to be the first one, so its position
/// is fixed and there is no chunk walk to do.
fn png(bytes: &[u8]) -> Option<ImageInfo> {
    if !bytes.starts_with(PNG_MAGIC) || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    sized("image/png", "png", be_u32(bytes, 16)?, be_u32(bytes, 20)?)
}

/// GIF's logical screen descriptor follows the six-byte signature.
fn gif(bytes: &[u8]) -> Option<ImageInfo> {
    let magic = bytes.get(..6)?;
    if magic != b"GIF87a" && magic != b"GIF89a" {
        return None;
    }
    sized("image/gif", "gif", le_u16(bytes, 6)?, le_u16(bytes, 8)?)
}

/// WebP is a RIFF container with three incompatible ways of stating a size:
/// lossy (`VP8 `), lossless (`VP8L`) and extended (`VP8X`). A file saved from a
/// browser can be any of them, so all three are read.
fn webp(bytes: &[u8]) -> Option<ImageInfo> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
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
            let packed =
                u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]);
            ((packed & 0x3fff) + 1, ((packed >> 14) & 0x3fff) + 1)
        }
        // Flags and reserved bits occupy the first four bytes of the payload;
        // the canvas size is stored minus one, so a 1x1 image reads as zero.
        b"VP8X" => (le_u24(bytes, 24)? + 1, le_u24(bytes, 27)? + 1),
        _ => return None,
    };
    sized("image/webp", "webp", width, height)
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

/// Walk the marker segments to the first SOF.
///
/// There is no fixed offset to read here: APP0/APP1/APP2 blocks of arbitrary
/// size come first, and a photograph out of a camera carries an EXIF preview
/// large enough that the frame header is many kilobytes in.
fn jpeg(bytes: &[u8]) -> Option<ImageInfo> {
    if bytes.get(..2)? != [0xff, 0xd8] {
        return None;
    }

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
            return sized("image/jpeg", "jpg", be_u16(bytes, at + 5)?, be_u16(bytes, at + 3)?);
        }
        at += length;
    }
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
        let mut payload = vec![0x10, 0, 0, 0];
        payload.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        payload.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        riff(b"VP8X", &payload)
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
}
