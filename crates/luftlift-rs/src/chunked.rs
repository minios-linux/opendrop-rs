//! HTTP/1.1 chunked transfer-encoding body decoder.
//!
//! The iOS-18 quirk that broke stock opendrop: `/Discover` and `/Ask` POSTs
//! arrive with `Transfer-Encoding: chunked` and **no** `Content-Length`. A
//! naive `int(Content-Length)` crashes. This module decodes the chunked body
//! (the bytes *after* the blank `\r\n` separating headers from body) into the
//! logical body bytes — a pure, I/O-free function for easy testing. See
//! `opendrop-patch/server.py.patched::_read_request_body` for the reference.
//!
//! Format (RFC 7230 §4.1):
//! ```text
//! chunked-body = *chunk last-chunk trailer-part CRLF
//! chunk        = chunk-size [ chunk-ext ] CRLF chunk-data CRLF
//! last-chunk   = 1*"0" [ chunk-ext ] CRLF
//! ```

use thiserror::Error;

/// Errors that can occur decoding a chunked body.
#[derive(Debug, PartialEq, Eq, Error)]
pub enum ChunkedError {
    #[error("chunk size line is not valid hex: {0:?}")]
    InvalidSizeLine(String),
    #[error("truncated body: needed {needed} more bytes after the size line")]
    Truncated { needed: usize },
    #[error("missing CRLF terminator")]
    MissingCrlf,
}

/// Decode an HTTP/1.1 chunked body into the logical body bytes.
///
/// `input` is the body portion *after* the header/data separator `\r\n\r\n`.
/// Stops at the last-chunk (`0\r\n`) and consumes any trailer lines. This is
/// the pure counterpart to the Python `_read_request_body` chunked branch.
pub fn decode_chunked(input: &[u8]) -> Result<Vec<u8>, ChunkedError> {
    let _ = input;
    // RED: unimplemented — tests below must fail first.
    Err(ChunkedError::Truncated { needed: 0 })
}

#[cfg(test)]
mod tests {
    use super::{ChunkedError, decode_chunked};

    #[test]
    fn decode_chunked_single_chunk() {
        // RFC example: "Wiki" in one chunk.
        let body = b"4\r\nWiki\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(body).unwrap(), b"Wiki");
    }

    #[test]
    fn decode_chunked_multiple_chunks() {
        // RFC 7230 §4.1 worked example.
        let body = b"4\r\nWiki\r\n6\r\npedia \r\n0\r\n\r\n";
        assert_eq!(decode_chunked(body).unwrap(), b"Wikipedia ");
    }

    #[test]
    fn decode_chunked_empty_body() {
        // No data, just the terminator — the no-Content-Length / empty case.
        let body = b"0\r\n\r\n";
        assert_eq!(decode_chunked(body).unwrap(), b"");
    }

    #[test]
    fn decode_chunked_ignores_chunk_extensions() {
        // chunk-ext after the size (e.g. ";foo=bar") must be ignored.
        let body = b"5;foo=bar\r\nhello\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(body).unwrap(), b"hello");
    }

    #[test]
    fn decode_chunked_handles_trailers() {
        // Optional trailer-part after the last-chunk.
        let body = b"3\r\nabc\r\n0\r\nX-Trace: 42\r\n\r\n";
        assert_eq!(decode_chunked(body).unwrap(), b"abc");
    }

    #[test]
    fn decode_chunked_preserves_binary_bytes() {
        // AirDrop bodies contain binary plists / zlib — byte-exact matters.
        let data: Vec<u8> = (0u8..=255).collect();
        let hex = format!("{:x}", data.len());
        let mut body = Vec::new();
        body.extend_from_slice(hex.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\r\n0\r\n\r\n");
        assert_eq!(decode_chunked(&body).unwrap(), data.as_slice());
    }

    #[test]
    fn decode_chunked_rejects_bad_hex_size() {
        let body = b"not-hex\r\nWiki\r\n0\r\n\r\n";
        assert!(matches!(
            decode_chunked(body),
            Err(ChunkedError::InvalidSizeLine(_))
        ));
    }

    #[test]
    fn decode_chunked_detects_truncated_data() {
        // Claims 10 bytes but only provides 4 before EOF.
        let body = b"10\r\nWiki";
        assert!(matches!(
            decode_chunked(body),
            Err(ChunkedError::Truncated { .. })
        ));
    }
}
