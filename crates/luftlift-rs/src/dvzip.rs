//! Decoder for Apple AirDrop's "dvzip" payload format
//! (Content-Type: `application/x-dvzip`), as sent by modern iOS
//! (observed on iOS 18.6.2). See `docs/airdrop-protocol.md` and the
//! authoritative Python reference at `opendrop-patch/dvzip.py`.
//!
//! Format: a sequence of framed zlib blocks
//!
//! ```text
//! repeat until end-of-data:
//!     u32  block_length   (big-endian)
//!     u8[block_length]    zlib stream (header 0x78 0x9c)
//! ```
//!
//! Concatenating the inflated output of every block yields an ODC CPIO
//! archive (magic ASCII `070707`).

use flate2::read::ZlibDecoder;
use std::io::Read;
use tracing::debug;

/// Decode a dvzip payload into the underlying CPIO archive bytes.
///
/// Matches `opendrop-patch/dvzip.py::dvzip_to_cpio`: walk framed zlib blocks,
/// inflate each, and concatenate. Stops cleanly on a trailing short frame or
/// a zero-length block (mirroring the Python loop guard
/// `block_len == 0 or pos + block_len > len(data)`).
///
/// Malformed zlib streams are skipped with a `debug!` log rather than a hard
/// error: we want best-effort recovery out of a captured body and the CPIO
/// magic check downstream will catch total garbage.
pub fn dvzip_to_cpio(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 4 <= data.len() {
        let block_len = u32::from_be_bytes([
            data[pos],
            data[pos + 1],
            data[pos + 2],
            data[pos + 3],
        ]) as usize;
        pos += 4;
        if block_len == 0 || pos + block_len > data.len() {
            break;
        }
        let raw = &data[pos..pos + block_len];
        pos += block_len;

        let mut decoder = ZlibDecoder::new(raw);
        let mut buf = Vec::new();
        match decoder.read_to_end(&mut buf) {
            Ok(_) => out.extend_from_slice(&buf),
            Err(e) => debug!(block_len, pos, error = %e, "dvzip: skipping malformed zlib block"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::dvzip_to_cpio;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    /// Frame a payload as a dvzip body: split it into the given chunk sizes,
    /// zlib-compress each chunk, and prefix each compressed stream with a
    /// big-endian u32 length — exactly the on-wire dvzip framing.
    fn frame_dvzip(cpio: &[u8], chunk_sizes: &[usize]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut pos = 0;
        for &sz in chunk_sizes {
            let end = (pos + sz).min(cpio.len());
            let chunk = &cpio[pos..end];
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(chunk).unwrap();
            let compressed = encoder.finish().unwrap();
            out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
            out.extend_from_slice(&compressed);
            pos = end;
        }
        assert_eq!(pos, cpio.len(), "chunk_sizes must cover the whole payload");
        out
    }

    /// A synthetic ODC-CPIO-shaped payload: starts with the `070707` magic so
    /// a future CPIO reader has something realistic to chew on. The dvzip
    /// decoder itself only cares about byte-exact round-trip.
    fn synthetic_cpio() -> Vec<u8> {
        let mut v = b"070707".to_vec();
        v.extend_from_slice(b"hello airdrop from luftlift-rs!");
        v
    }

    #[test]
    fn dvzip_decodes_single_framed_zlib_block_to_cpio() {
        let cpio = synthetic_cpio();
        let framed = frame_dvzip(&cpio, &[cpio.len()]);

        let decoded = dvzip_to_cpio(&framed);
        assert_eq!(&decoded[..6], b"070707", "decoded bytes must be CPIO magic");
        assert_eq!(decoded, cpio, "round-trip must recover the original bytes");
    }

    #[test]
    fn dvzip_decodes_multiple_framed_zlib_blocks_to_cpio() {
        let cpio = synthetic_cpio();
        // iOS-18 captures typically span several zlib blocks (the real
        // /tmp/upload.bin fixture is 4 blocks). Mix a sub-payload split with a
        // final block that exactly exhausts the buffer.
        let framed = frame_dvzip(&cpio, &[6, 7, cpio.len() - 13]);

        let decoded = dvzip_to_cpio(&framed);
        assert_eq!(&decoded[..6], b"070707");
        assert_eq!(decoded, cpio);
    }

    #[test]
    fn dvzip_empty_input_yields_empty_output() {
        assert_eq!(dvzip_to_cpio(&[]), Vec::<u8>::new());
    }
}
