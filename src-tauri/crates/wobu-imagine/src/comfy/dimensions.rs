//! The size an image actually is, read off its header.
//!
//! The trait's fifth contract point: "read the dimensions back off the image,
//! rather than echoing the ones that were requested". It reads as belt and
//! braces for a local backend that was handed explicit width and height — until
//! a workflow with a hires-fix pass or a `ImageScaleBy` node returns something
//! larger than the latent it started from, and `Asset.width`/`height` record a
//! number that never existed. Every thumbnail generated from those is stretched,
//! and nothing fails.
//!
//! Headers rather than a decoder, and so no `image` or `png` dependency. The
//! whole question is "what does the container say its size is", which lives in
//! the first few dozen bytes of all three formats ComfyUI can write; decoding
//! the pixels to find out would mean pulling megabytes of codec into a crate
//! whose own documentation says it does no decoding.

/// Width and height, or `None` when the bytes are not a picture we recognise.
///
/// `None` becomes [`Error::NotAnImage`](crate::Error::NotAnImage), which
/// `error.rs` puts on the "the call succeeded and what came back is not
/// something we can keep" side of the line — the right side, because a local
/// render that produced bytes has already spent the GPU time.
pub(crate) fn read(bytes: &[u8]) -> Option<(u32, u32)> {
    png(bytes).or_else(|| jpeg(bytes)).or_else(|| webp(bytes))
}

/// The mime type those bytes are, for `Asset.mime` and for the webview.
pub(crate) fn mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        "image/jpeg"
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        // Never reached for anything `read` accepted, and stated rather than
        // panicked on: a caller that took the mime without the dimensions would
        // otherwise be one `expect` away from taking down a render.
        "application/octet-stream"
    }
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// PNG puts `IHDR` immediately after the signature, so the size is at a fixed
/// offset. ComfyUI's `SaveImage` writes PNG.
fn png(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((be32(bytes, 16)?, be32(bytes, 20)?))
}

/// JPEG has no fixed offset: the size is in whichever start-of-frame marker the
/// encoder used, after however many metadata segments it wrote first. Latent
/// previews are JPEG, and so is any workflow ending in `SaveImage` with a JPEG
/// node in front of it.
fn jpeg(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut at = 2;
    loop {
        // Segments are separated by any number of `0xff` fill bytes, which some
        // encoders emit and most do not.
        while bytes.get(at) == Some(&0xff) {
            at += 1;
        }
        let marker = *bytes.get(at)?;
        at += 1;
        match marker {
            // Standalone markers carry no length. `0xd9` ends the file; reaching
            // it means there was no frame header, which is a truncated image.
            0x01 | 0xd0..=0xd9 => continue,
            // Every start-of-frame except the four that are not frames at all.
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                let height = u16::from_be_bytes(bytes.get(at + 3..at + 5)?.try_into().ok()?);
                let width = u16::from_be_bytes(bytes.get(at + 5..at + 7)?.try_into().ok()?);
                return Some((u32::from(width), u32::from(height)));
            }
            _ => {
                let length = u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?);
                // A zero-length segment would loop forever on a corrupt file,
                // and the loop is reading bytes off a network.
                at = at.checked_add(usize::from(length).max(2))?;
                // `0xff` is where the next marker begins, and a segment whose
                // declared length walked us into the middle of one is corrupt.
                if bytes.get(at) != Some(&0xff) {
                    return None;
                }
            }
        }
    }
}

/// WebP, in all three of its chunk layouts. Little-endian, unlike the other two,
/// and the extended form stores the sizes minus one across three bytes each.
fn webp(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 30 || !bytes.starts_with(b"RIFF") || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    let le16 = |at: usize| -> Option<u32> {
        Some(u32::from(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?)))
    };
    match bytes.get(12..16)? {
        b"VP8 " => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
        b"VP8L" => {
            let bits = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => {
            let three = |at: usize| -> Option<u32> {
                Some(
                    u32::from(*bytes.get(at)?)
                        | u32::from(*bytes.get(at + 1)?) << 8
                        | u32::from(*bytes.get(at + 2)?) << 16,
                )
            };
            Some((three(24)? + 1, three(27)? + 1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG header with the given size. Only the signature and `IHDR` matter to
    /// `read`, and writing the rest would mean a real encoder.
    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    /// A JPEG with `count` metadata segments in front of the frame header, which
    /// is what an encoder that writes EXIF and a colour profile produces.
    fn jpeg_bytes(width: u16, height: u16, count: usize) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8];
        for _ in 0..count {
            bytes.extend_from_slice(&[0xff, 0xe1, 0x00, 0x08, 1, 2, 3, 4, 5, 6]);
        }
        bytes.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 8]);
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes
    }

    #[test]
    fn a_png_reports_the_size_in_its_header_and_not_the_one_that_was_requested() {
        // The trait's fifth contract point. A workflow with a hires pass returns
        // something larger than the latent it started from, and `Asset.width`
        // recorded from the request would be a number that never existed —
        // every thumbnail built from it is stretched and nothing fails.
        assert_eq!(read(&png_bytes(2048, 1152)), Some((2048, 1152)));
        assert_eq!(mime(&png_bytes(1, 1)), "image/png");
        assert_eq!(read(&png_bytes(1, 1)), Some((1, 1)));
    }

    #[test]
    fn a_jpeg_is_found_however_much_metadata_the_encoder_wrote_first() {
        // JPEG has no fixed offset for the size: it is in whichever start-of-frame
        // marker follows however many EXIF and colour-profile segments came
        // first. Reading a fixed offset works on the file you tested with and on
        // no other.
        for segments in [0, 1, 5] {
            assert_eq!(read(&jpeg_bytes(1024, 768, segments)), Some((1024, 768)), "{segments}");
        }
        assert_eq!(mime(&jpeg_bytes(8, 8, 0)), "image/jpeg");
    }

    #[test]
    fn a_progressive_jpeg_is_read_and_a_huffman_table_is_not_mistaken_for_a_frame() {
        // `0xc4` sits inside the start-of-frame marker range and is a Huffman
        // table, not a frame. Treating it as one reads two bytes of a coding
        // table as the image size.
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xc4, 0x00, 0x06, 1, 2, 3, 4];
        // `0xc2` is a progressive frame, which is what a size-optimised encoder
        // emits and what a naive `0xc0`-only reader misses.
        bytes.extend_from_slice(&[0xff, 0xc2, 0x00, 0x11, 8, 0x02, 0x00, 0x03, 0x00]);
        bytes.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        assert_eq!(read(&bytes), Some((768, 512)));
    }

    #[test]
    fn bytes_that_are_not_an_image_are_reported_rather_than_guessed_at() {
        // What `/view` returns when the file has been swept up behind us, or
        // when a proxy answered instead. Guessing a size here would write a
        // plausible number into an asset record for bytes that are an HTML
        // error page.
        assert_eq!(read(b"<!DOCTYPE html><html>404"), None);
        assert_eq!(read(b""), None);
        assert_eq!(read(&png_bytes(4, 4)[..15]), None, "a truncated header is not a size");
        assert_eq!(mime(b"not an image"), "application/octet-stream");
    }

    #[test]
    fn a_corrupt_jpeg_stops_rather_than_scanning_forever() {
        // The loop is reading bytes off a socket, so a segment whose declared
        // length is zero or walks past the end must terminate. It hangs a
        // generation otherwise, with the queue holding a slot.
        assert_eq!(read(&[0xff, 0xd8, 0xff, 0xe1, 0x00, 0x00, 1, 2]), None);
        assert_eq!(read(&[0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff]), None);
        assert_eq!(read(&[0xff, 0xd8, 0xff, 0xd9]), None);
    }

    #[test]
    fn all_three_webp_layouts_report_their_size() {
        // ComfyUI can write WebP, and it writes the extended form when the image
        // has an alpha channel — which a render on a transparent background does.
        let mut lossy = b"RIFF\x00\x00\x00\x00WEBPVP8 ".to_vec();
        lossy.extend_from_slice(&[0; 10]);
        lossy.extend_from_slice(&512u16.to_le_bytes());
        lossy.extend_from_slice(&384u16.to_le_bytes());
        assert_eq!(read(&lossy), Some((512, 384)));

        let mut extended = b"RIFF\x00\x00\x00\x00WEBPVP8X".to_vec();
        extended.extend_from_slice(&[0x0a, 0, 0, 0, 0x10, 0, 0, 0]);
        // Both sizes are stored minus one, across three little-endian bytes.
        extended.extend_from_slice(&[0xff, 0x03, 0x00]);
        extended.extend_from_slice(&[0x7f, 0x02, 0x00]);
        extended.extend_from_slice(&[0; 8]);
        assert_eq!(read(&extended), Some((1024, 640)));
        assert_eq!(mime(&extended), "image/webp");
    }
}
