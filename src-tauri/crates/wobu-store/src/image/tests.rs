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
    for (tag, expected) in [(1, (800, 600)), (2, (800, 600)), (3, (800, 600)), (4, (800, 600))] {
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
    assert_eq!(probe(&jpeg_with_app1(640, 480, &liar)).unwrap().orientation, Orientation::Normal,);
}
