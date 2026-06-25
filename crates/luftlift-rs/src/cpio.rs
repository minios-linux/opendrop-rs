//! Reader for the POSIX ODC CPIO archive format (old portable ASCII,
//! magic ASCII `070707`) — the inner format produced by decoding an
//! AirDrop dvzip payload (see `docs/airdrop-protocol.md` and the `dvzip`
//! module). Extracted entries are the transferred file(s); for a link
//! share the payload contains a `.webloc` file.
//!
//! ## ODC header layout (76 bytes, all ASCII octal)
//! ```text
//! offset  len  field
//! 0       6    c_magic     "070707"
//! 6       6    c_dev
//! 12      6    c_ino
//! 18      6    c_mode
//! 24      6    c_uid
//! 30      6    c_gid
//! 36      6    c_nlink
//! 42      6    c_rdev
//! 48      11   c_mtime
//! 59      6    c_namesize  (includes the trailing NUL)
//! 65      11   c_filesize
//! ```
//! The header is followed by `c_namesize` name bytes (NUL-terminated) and
//! then `c_filesize` data bytes — with **no** alignment padding in ODC
//! (unlike the "newc" `070701` format). The archive is terminated by an
//! entry named `TRAILER!!!` with a zero-length data body.

/// A single file entry extracted from an ODC CPIO archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpioEntry<'a> {
    /// Entry name with the trailing NUL stripped.
    pub name: String,
    /// Raw file bytes, borrowed from the input archive.
    pub data: &'a [u8],
}

/// Fixed width of an ODC CPIO header (ASCII octal fields, no padding).
pub const HDR_LEN: usize = 76;
/// ODC magic bytes (ASCII `070707`).
pub const MAGIC: &[u8] = b"070707";

const NAMESIZE_OFF: usize = 59;
const NAMESIZE_LEN: usize = 6;
const FILESIZE_OFF: usize = 65;
const FILESIZE_LEN: usize = 11;
const TRAILER_NAME: &str = "TRAILER!!!";

/// Extract entries from an ODC CPIO archive (magic `070707`).
///
/// Walks 76-byte ASCII-octal headers, emitting each entry's name + data.
/// Stops at the `TRAILER!!!` entry (which is not emitted) or at a short /
/// malformed frame. A missing or wrong magic on the first header yields no
/// entries — the downstream caller is expected to treat an empty result on
/// a non-empty input as "not a CPIO".
pub fn read_odc_cpio(_data: &[u8]) -> Vec<CpioEntry<'_>> {
    // RED: unimplemented — the round-trip tests below must fail first.
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::{CpioEntry, HDR_LEN, MAGIC, read_odc_cpio};

    /// Build a single 76-byte ODC header as ASCII. Asserts its own length so
    /// a field-offset regression in the test scaffolding is caught loudly.
    fn build_header(ino: u32, namesize: usize, filesize: usize) -> String {
        let mut s = String::new();
        s.push_str("070707"); // c_magic
        s.push_str("000000"); // c_dev
        s.push_str(&format!("{:06o}", ino & 0o777777)); // c_ino
        s.push_str("100644"); // c_mode  (regular file, 0644)
        s.push_str("000000"); // c_uid
        s.push_str("000000"); // c_gid
        s.push_str("000001"); // c_nlink
        s.push_str("000000"); // c_rdev
        s.push_str("00000000000"); // c_mtime (11 octal digits)
        s.push_str(&format!("{:06o}", namesize)); // c_namesize
        s.push_str(&format!("{:011o}", filesize)); // c_filesize
        assert_eq!(s.len(), HDR_LEN, "ODC header must be exactly 76 bytes");
        s
    }

    /// Encode a sequence of (name, data) entries as an ODC CPIO archive,
    /// optionally appending the `TRAILER!!!` terminator real writers emit.
    fn encode_odc_cpio(entries: &[(&str, &[u8])], with_trailer: bool) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, &(name, data)) in entries.iter().enumerate() {
            let name_with_nul = format!("{}\0", name);
            let header = build_header((i + 1) as u32, name_with_nul.len(), data.len());
            out.extend_from_slice(header.as_bytes());
            out.extend_from_slice(name_with_nul.as_bytes());
            out.extend_from_slice(data);
        }
        if with_trailer {
            let trailer = "TRAILER!!!\0";
            let header = build_header(0, trailer.len(), 0);
            out.extend_from_slice(header.as_bytes());
            out.extend_from_slice(trailer.as_bytes());
        }
        out
    }

    #[test]
    fn read_odc_cpio_decodes_single_handbuilt_entry() {
        // Ground-truth archive built without the encoder: one entry "a" with
        // empty data. Header is exactly 76 ASCII bytes; then "a\0".
        let mut archive = Vec::new();
        archive.extend_from_slice(build_header(1, 2, 0).as_bytes()); // namesize=2 ("a\0")
        archive.extend_from_slice(b"a\0");
        assert_eq!(archive.len(), HDR_LEN + 2);

        let entries = read_odc_cpio(&archive);
        assert_eq!(
            entries,
            vec![CpioEntry {
                name: "a".to_string(),
                data: b"",
            }]
        );
    }

    #[test]
    fn read_odc_cpio_roundtrips_multiple_entries() {
        let jpeg_ish = [0xFFu8, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46];
        let entries_in: &[(&str, &[u8])] = &[
            ("hello.txt", b"hi from luftlift"),
            ("FullSizeRender.jpg", &jpeg_ish),
            ("dir/nested.bin", &[0x00, 0xFF, 0x7F, 0x80, 0x01]),
        ];
        let archive = encode_odc_cpio(entries_in, true);

        let got = read_odc_cpio(&archive);
        assert_eq!(got.len(), entries_in.len());
        for (expected, actual) in entries_in.iter().zip(got.iter()) {
            assert_eq!(actual.name, expected.0);
            assert_eq!(actual.data, expected.1);
        }
    }

    #[test]
    fn read_odc_cpio_stops_at_trailer_and_omits_it() {
        let entries_in: &[(&str, &[u8])] = &[("only.txt", b"x")];
        let archive = encode_odc_cpio(entries_in, true);

        let got = read_odc_cpio(&archive);
        assert_eq!(got.len(), 1, "TRAILER!!! must not appear as an entry");
        assert_eq!(got[0].name, "only.txt");
        assert_eq!(got[0].data, b"x");
    }

    #[test]
    fn read_odc_cpio_preserves_exact_binary_bytes() {
        // A byte sweep including the tricky values (0x00, 0xFF, 0x0A newline,
        // 0x00 NUL inside *data* — distinct from name NULs).
        let data: Vec<u8> = (0u8..=255).collect();
        let archive = encode_odc_cpio(&[("sweep.bin", &data)], true);

        let got = read_odc_cpio(&archive);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].data, data.as_slice());
    }

    #[test]
    fn read_odc_cpio_empty_archive_yields_no_entries() {
        assert!(read_odc_cpio(&[]).is_empty());
    }

    #[test]
    fn read_odc_cpio_non_magic_input_yields_no_entries() {
        // Bytes that are not an ODC CPIO: must not be misread as one.
        let bogus = b"this is definitely not a cpio archive at all";
        assert!(read_odc_cpio(bogus).is_empty());
        // Sanity: the magic constant is what we expect on the wire.
        assert_eq!(MAGIC, b"070707");
    }
}
