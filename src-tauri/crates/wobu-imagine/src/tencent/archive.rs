//! Opening the `.zip` the OBJ result URL actually points at.
//!
//! `docs/08-providers.md`: "the OBJ `Url` is a **`.zip`** (mesh + `.mtl` +
//! texture maps), not a bare mesh, so the downloader must unzip". A downloader
//! that writes those bytes to `model.obj` produces a file every viewer refuses,
//! and the mistake is invisible until somebody tries to open it — which is
//! usually after the 24-hour result URL has expired and the job cannot be
//! fetched again.
//!
//! ## Why this is a hundred lines rather than a dependency
//!
//! `flate2` is already in the workspace lock at 1.1.9 (by way of `png`) and
//! brings the DEFLATE decoder and a CRC-32 with it. A ZIP reader on top of that
//! is the central directory, the local headers and two compression methods; a
//! `zip` crate would add half a dozen entries to the lock for the same two, and
//! `Cargo.toml`'s existing notes are explicit that a second copy of something the
//! tree already has is a cost worth avoiding.
//!
//! What is deliberately **not** supported, each refused with a sentence rather
//! than mis-read:
//!
//! - **Encrypted entries.** Nothing Tencent produces is, and a decryptor that
//!   silently returned the ciphertext would write a texture of noise.
//! - **ZIP64.** It begins above 4 GiB, which a concept mesh is not, and half an
//!   implementation is worse than none: the 32-bit fields hold `0xFFFFFFFF` as a
//!   sentinel, and a reader that took that literally would try to allocate it.
//! - **Compression methods other than stored and deflate.** Refused by name so
//!   the next person knows which one to add.
//!
//! ## Read from the central directory, not by walking local headers
//!
//! Both are common; only one is correct. A local header written with the
//! data-descriptor flag carries **zeroes** for the compressed and uncompressed
//! sizes — the real values follow the data — so a reader that walks forward from
//! the start of the file has no way to know where any entry ends. The central
//! directory at the tail always carries the real sizes. Local headers are still
//! read, but only for their name and extra lengths, because those can differ from
//! the central copy and they are what put the data offset in the right place.

use std::io::Read;

use flate2::Crc;
use flate2::read::DeflateDecoder;

use crate::error::Error;
use crate::mesh::MeshFile;

/// The signature at the start of the end-of-central-directory record.
const EOCD: u32 = 0x0605_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const LOCAL_HEADER: u32 = 0x0403_4b50;

/// The first four bytes of every non-empty ZIP.
///
/// Used to decide whether a download *is* an archive, which is a fact about the
/// bytes rather than about the `Type` string the provider declared —
/// `docs/08-providers.md` warns those two disagree.
pub(crate) const ZIP_MAGIC: [u8; 4] = [b'P', b'K', 0x03, 0x04];

/// The magic at the start of a binary glTF container.
pub(crate) const GLB_MAGIC: [u8; 4] = *b"glTF";

const STORED: u16 = 0;
const DEFLATE: u16 = 8;

/// Set in the general-purpose flags when the entry is encrypted.
const ENCRYPTED: u16 = 1 << 0;

/// The sentinel the 32-bit size and offset fields carry when the real value is
/// in a ZIP64 extra field.
const ZIP64_SENTINEL: u32 = 0xFFFF_FFFF;

/// The most we will inflate out of one archive.
///
/// A 1.5-million-face mesh with PBR texture maps is a few hundred megabytes at
/// the very top of the range, so this is generous. It exists because the sizes
/// driving the allocation are numbers in a file we downloaded: without a cap, an
/// archive declaring four gigabytes of output gets four gigabytes of allocation
/// before anything notices, and the process is gone.
const MAX_INFLATED: u64 = 2 << 30;

/// Whether these bytes are a ZIP.
pub(crate) fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(&ZIP_MAGIC)
}

/// Whether these bytes are a binary glTF.
pub(crate) fn is_glb(bytes: &[u8]) -> bool {
    bytes.starts_with(&GLB_MAGIC)
}

/// Every file in the archive, in central-directory order.
///
/// Directory entries are dropped — they carry no bytes and a store writing them
/// would create empty files named after folders. Everything else is kept,
/// including things we do not recognise: an archive from a future release with a
/// `readme.txt` in it is not a reason to fail a job that has been paid for.
pub(crate) fn unpack(bytes: &[u8]) -> Result<Vec<MeshFile>, Error> {
    let directory = find_eocd(bytes)?;
    let mut at = directory.offset;
    let mut files = Vec::with_capacity(directory.entries);
    let mut inflated: u64 = 0;

    for _ in 0..directory.entries {
        let entry = read_central(bytes, at)?;
        at = entry.next;
        if entry.name.ends_with('/') {
            continue;
        }
        inflated = inflated.saturating_add(u64::from(entry.uncompressed));
        if inflated > MAX_INFLATED {
            return Err(broken(format!(
                "the archive declares more than {} GiB of contents, which no mesh is",
                MAX_INFLATED >> 30,
            )));
        }
        files.push(MeshFile::new(entry.name.clone(), read_data(bytes, &entry)?));
    }

    if files.is_empty() {
        return Err(broken("the archive is empty".to_owned()));
    }
    Ok(files)
}

/// Where the central directory is, read from the tail.
struct Directory {
    offset: usize,
    entries: usize,
}

/// The end-of-central-directory record, found by searching backwards.
///
/// Backwards because the record ends with a variable-length comment, so its
/// position is not a fixed distance from the end. The search is bounded to the
/// 64 KiB a comment length can express plus the record itself — an unbounded
/// scan over a several-hundred-megabyte download looking for four bytes that
/// occur naturally in texture data is both slow and more likely to find the wrong
/// one.
fn find_eocd(bytes: &[u8]) -> Result<Directory, Error> {
    const FIXED: usize = 22;
    if bytes.len() < FIXED {
        return Err(broken(format!("only {} bytes arrived, which is not an archive", bytes.len())));
    }
    let horizon = bytes.len().saturating_sub(FIXED + u16::MAX as usize);
    let start = (horizon..=bytes.len() - FIXED)
        .rev()
        .find(|at| u32_at(bytes, *at) == Some(EOCD))
        .ok_or_else(|| {
            broken(
                "the download begins like a ZIP and has no end-of-central-directory record, so it \
                 is truncated"
                    .to_owned(),
            )
        })?;

    let entries = u16_at(bytes, start + 10).unwrap_or(0) as usize;
    let offset = u32_at(bytes, start + 16).unwrap_or(0);
    if offset == ZIP64_SENTINEL || entries == u16::MAX as usize {
        return Err(unsupported("ZIP64"));
    }
    let offset = offset as usize;
    if offset >= bytes.len() {
        return Err(broken("the central directory points past the end of the download".to_owned()));
    }
    Ok(Directory { offset, entries })
}

/// One central-directory entry, with everything needed to find and check its
/// data.
struct Entry {
    name: String,
    method: u16,
    crc: u32,
    compressed: u32,
    uncompressed: u32,
    local: usize,
    /// Where the next central-directory header starts.
    next: usize,
}

fn read_central(bytes: &[u8], at: usize) -> Result<Entry, Error> {
    const FIXED: usize = 46;
    if u32_at(bytes, at) != Some(CENTRAL_HEADER) {
        return Err(broken("the central directory is not where the archive says it is".to_owned()));
    }
    let short = |offset: usize| u16_at(bytes, at + offset).unwrap_or(0);
    let long = |offset: usize| u32_at(bytes, at + offset).unwrap_or(0);

    if short(8) & ENCRYPTED != 0 {
        return Err(unsupported("an encrypted entry"));
    }
    let name_len = short(28) as usize;
    let extra_len = short(30) as usize;
    let comment_len = short(32) as usize;
    let name = bytes
        .get(at + FIXED..at + FIXED + name_len)
        .ok_or_else(|| broken("an entry name runs past the end of the download".to_owned()))?;
    let name = safe_name(String::from_utf8_lossy(name).as_ref())?;

    let compressed = long(20);
    let uncompressed = long(24);
    let local = long(42);
    if compressed == ZIP64_SENTINEL || uncompressed == ZIP64_SENTINEL || local == ZIP64_SENTINEL {
        return Err(unsupported("ZIP64"));
    }

    Ok(Entry {
        name,
        method: short(10),
        crc: long(16),
        compressed,
        uncompressed,
        local: local as usize,
        next: at + FIXED + name_len + extra_len + comment_len,
    })
}

/// The entry's bytes, decompressed and checked.
///
/// The local header is re-read for its own name and extra lengths, which are
/// allowed to differ from the central directory's and frequently do — writers put
/// a timestamp extra field in one and not the other. Computing the data offset
/// from the central copy lands a few bytes into the file's own name.
fn read_data(bytes: &[u8], entry: &Entry) -> Result<Vec<u8>, Error> {
    const FIXED: usize = 30;
    if u32_at(bytes, entry.local) != Some(LOCAL_HEADER) {
        return Err(broken(format!("`{}` does not start where the archive says it does", entry.name)));
    }
    let name_len = u16_at(bytes, entry.local + 26).unwrap_or(0) as usize;
    let extra_len = u16_at(bytes, entry.local + 28).unwrap_or(0) as usize;
    let from = entry.local + FIXED + name_len + extra_len;
    let data = bytes
        .get(from..from + entry.compressed as usize)
        .ok_or_else(|| broken(format!("`{}` runs past the end of the download", entry.name)))?;

    let out = match entry.method {
        STORED => data.to_vec(),
        DEFLATE => {
            let mut out = Vec::with_capacity(entry.uncompressed as usize);
            // Bounded by the declared size rather than by trust: the length is a
            // number from the file, and a stream that keeps producing past it is
            // the shape of a decompression bomb.
            DeflateDecoder::new(data)
                .take(u64::from(entry.uncompressed) + 1)
                .read_to_end(&mut out)
                .map_err(|e| broken(format!("`{}` would not decompress: {e}", entry.name)))?;
            out
        }
        other => return Err(unsupported(&format!("compression method {other} (in `{}`)", entry.name))),
    };

    if out.len() as u64 != u64::from(entry.uncompressed) {
        return Err(broken(format!(
            "`{}` unpacked to {} bytes and the archive says {}",
            entry.name,
            out.len(),
            entry.uncompressed,
        )));
    }
    // The CRC is the only thing that distinguishes "we decompressed it" from "we
    // decompressed it correctly", and a texture that is subtly wrong is a mesh
    // that looks broken for a reason nobody can find.
    let mut crc = Crc::new();
    crc.update(&out);
    if crc.sum() != entry.crc {
        return Err(broken(format!("`{}` did not survive the download intact", entry.name)));
    }
    Ok(out)
}

/// A name safe to join onto a directory.
///
/// These names come off the network and are handed to a store that writes them
/// to disk next to each other, because an `.obj` references its `.mtl` by name.
/// An entry called `../../.ssh/authorized_keys` is the oldest trick there is, and
/// the fact that we trust the provider is not the same as trusting whatever
/// produced the archive it is serving.
fn safe_name(raw: &str) -> Result<String, Error> {
    let name = raw.replace('\\', "/");
    let unsafe_component = name.starts_with('/')
        || name.split('/').any(|part| part == ".." || part.contains(':'))
        || name.is_empty();
    if unsafe_component {
        return Err(broken(format!("the archive contains an entry named `{raw}`")));
    }
    Ok(name)
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn broken(detail: String) -> Error {
    Error::NotAMesh { detail }
}

/// A ZIP feature we do not read.
///
/// Named rather than reported as corruption, because the two send whoever reads
/// the log to completely different places: one is a download to retry and the
/// other is a hundred lines to write.
fn unsupported(feature: &str) -> Error {
    Error::NotAMesh {
        detail: format!("the archive uses {feature}, which wobu's reader does not support"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::DeflateEncoder;
    use std::io::Write;

    /// Build an archive, so the reader can be driven without one on disk.
    ///
    /// Written here rather than pulled in, for the same reason the reader is:
    /// a writer dependency to test a reader that exists to avoid a dependency
    /// would be a strange trade. `deflate` picks the method, which is what lets
    /// the same fixture exercise both branches.
    pub(crate) fn zip(entries: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();

        for (name, contents) in entries {
            let mut crc = Crc::new();
            crc.update(contents);
            let (method, data) = if deflate {
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(contents).unwrap();
                (DEFLATE, encoder.finish().unwrap())
            } else {
                (STORED, contents.to_vec())
            };
            let local = out.len() as u32;

            out.extend_from_slice(&LOCAL_HEADER.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&crc.sum().to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(contents.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            // A local extra field the central directory does not have, which is
            // exactly the case a reader that trusts the central lengths gets
            // wrong.
            out.extend_from_slice(&4u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
            out.extend_from_slice(&data);

            central.extend_from_slice(&CENTRAL_HEADER.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&crc.sum().to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(contents.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&local.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }

        let directory_at = out.len() as u32;
        let directory_len = central.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&directory_len.to_le_bytes());
        out.extend_from_slice(&directory_at.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    /// What the OBJ result URL delivers, per `docs/08-providers.md`.
    pub(crate) fn obj_archive() -> Vec<u8> {
        zip(
            &[
                ("model.obj", b"mtllib model.mtl\nv 0.0 0.0 0.0\n" as &[u8]),
                ("model.mtl", b"newmtl material_0\nmap_Kd texture_0.png\n"),
                ("texture_0.png", &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
            ],
            true,
        )
    }

    #[test]
    fn the_obj_result_is_an_archive_of_three_files_and_not_a_mesh() {
        // The trap `docs/08-providers.md` names: writing these bytes to
        // `model.obj` produces a file every viewer refuses, and nothing notices
        // until somebody opens it — by which time the 24-hour URL has expired and
        // the job cannot be fetched again.
        let bytes = obj_archive();
        assert!(is_zip(&bytes), "and it announces itself in its first four bytes");
        assert!(!is_glb(&bytes));

        let files = unpack(&bytes).unwrap();
        let names: Vec<&str> = files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(names, ["model.obj", "model.mtl", "texture_0.png"]);
        assert!(files[0].bytes.starts_with(b"mtllib model.mtl"));
        assert_eq!(files[2].bytes, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    }

    #[test]
    fn a_stored_entry_and_a_deflated_one_both_come_out_the_same() {
        // Text deflates and a PNG usually does not, so a real archive contains
        // both methods. A reader that handled only one would return an `.obj` of
        // compressed noise, which is a file that opens and is wrong.
        let contents: &[u8] = b"v 1.0 2.0 3.0\nv 4.0 5.0 6.0\nf 1 2 3\n";
        for deflate in [false, true] {
            let files = unpack(&zip(&[("model.obj", contents)], deflate)).unwrap();
            assert_eq!(files[0].bytes, contents, "deflate={deflate}");
        }
    }

    #[test]
    fn the_data_offset_is_computed_from_the_local_header_and_not_the_central_one() {
        // The bug this guards: the extra field lengths in the two headers are
        // allowed to differ and routinely do, so a reader that adds the central
        // directory's `extra_len` to the local header lands a few bytes inside
        // the file and returns something that decompresses to nothing. The
        // fixture writes a four-byte local extra and none centrally.
        let files = unpack(&zip(&[("model.obj", b"v 0 0 0\n")], false)).unwrap();
        assert_eq!(files[0].bytes, b"v 0 0 0\n");
    }

    #[test]
    fn an_entry_that_did_not_survive_the_download_is_reported_rather_than_kept() {
        // The CRC is the only thing separating "we decompressed it" from "we
        // decompressed it correctly". A subtly wrong texture is a mesh that looks
        // broken for a reason nobody can find, hours later, in another program.
        let mut bytes = zip(&[("model.obj", b"v 0 0 0\n")], false);
        let at = bytes.windows(8).position(|w| w == b"v 0 0 0\n").unwrap();
        bytes[at] = b'x';
        let error = unpack(&bytes).unwrap_err();
        assert!(error.to_string().contains("did not survive"), "{error}");
        assert!(matches!(error, Error::NotAMesh { .. }));
    }

    #[test]
    fn an_entry_that_would_escape_the_folder_it_is_written_into_is_refused() {
        // These names come off the network and are handed to a store that writes
        // them next to each other, because an `.obj` names its `.mtl` by string.
        // Trusting the provider is not the same as trusting whatever built the
        // archive it is serving.
        for hostile in ["../../.ssh/authorized_keys", "/etc/passwd", "..\\..\\evil.obj"] {
            let error = unpack(&zip(&[(hostile, b"x" as &[u8])], false)).unwrap_err();
            assert!(error.to_string().contains(hostile), "{error}");
        }
        // And an ordinary subfolder still works, because real archives have them.
        let files = unpack(&zip(&[("textures/albedo.png", b"x" as &[u8])], false)).unwrap();
        assert_eq!(files[0].name, "textures/albedo.png");
    }

    #[test]
    fn a_feature_we_do_not_read_says_so_rather_than_looking_like_corruption() {
        // An unsupported method and a truncated download send whoever reads the
        // log to two different places: one is a retry, the other is a hundred
        // lines to write. Reported identically, the second gets retried forever.
        let mut bytes = zip(&[("model.obj", b"v 0 0 0\n")], false);
        // Method 12 is bzip2, at its offset in the central header.
        let central = bytes.windows(4).rposition(|w| w == CENTRAL_HEADER.to_le_bytes()).unwrap();
        bytes[central + 10] = 12;
        let error = unpack(&bytes).unwrap_err();
        assert!(error.to_string().contains("compression method 12"), "{error}");
        assert!(error.to_string().contains("does not support"), "{error}");

        let truncated = unpack(&[b'P', b'K', 3, 4, 0, 0, 0, 0]).unwrap_err();
        assert!(truncated.to_string().contains("not an archive"), "{truncated}");
    }

    #[test]
    fn an_encrypted_entry_is_refused_rather_than_written_out_as_ciphertext() {
        // A decryptor we do not have, failing open, is a texture of noise and a
        // mesh that renders as static — with every byte accounted for and no
        // error anywhere.
        let mut bytes = zip(&[("model.obj", b"v 0 0 0\n")], false);
        let central = bytes.windows(4).rposition(|w| w == CENTRAL_HEADER.to_le_bytes()).unwrap();
        bytes[central + 8] = ENCRYPTED as u8;
        let error = unpack(&bytes).unwrap_err();
        assert!(error.to_string().contains("encrypted"), "{error}");
    }

    #[test]
    fn a_zip64_archive_is_named_rather_than_read_as_a_four_gigabyte_allocation() {
        // The 32-bit fields carry `0xFFFFFFFF` as a sentinel meaning "the real
        // value is in an extra field". A reader that took it literally would try
        // to allocate four gigabytes for one entry.
        let mut bytes = zip(&[("model.obj", b"v 0 0 0\n")], false);
        let central = bytes.windows(4).rposition(|w| w == CENTRAL_HEADER.to_le_bytes()).unwrap();
        bytes[central + 24..central + 28].copy_from_slice(&ZIP64_SENTINEL.to_le_bytes());
        let error = unpack(&bytes).unwrap_err();
        assert!(error.to_string().contains("ZIP64"), "{error}");
    }

    #[test]
    fn a_glb_is_recognised_by_its_own_magic_and_not_by_the_declared_type() {
        // `docs/08-providers.md`: the international docs contradict GLB being
        // returned, so what a download *is* has to be read off the bytes. The two
        // magics are four bytes each and cannot both match.
        let glb = [&GLB_MAGIC[..], &2u32.to_le_bytes()[..]].concat();
        assert!(is_glb(&glb));
        assert!(!is_zip(&glb));
        assert!(!is_glb(&obj_archive()));
        assert!(!is_glb(b"") && !is_zip(b""));
    }
}
