//! Minimal HTTP/1.1 request parser + response formatter for the AirDrop
//! endpoints (`/Discover`, `/Ask`, `/Upload`). Kept as pure functions over
//! byte buffers so the routing/body-decoding logic is fully unit-testable
//! without sockets or TLS. The iOS-18 quirk (chunked bodies with no
//! Content-Length) is handled via the [`crate::chunked`] decoder.
//!
//! Reference: `opendrop-patch/server.py.patched` (`do_POST`, `_read_request_body`).

use crate::chunked::decode_chunked;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum HttpError {
    #[error("malformed request line")]
    BadRequestLine,
    #[error("header section too short / no terminator")]
    NoHeaderTerminator,
    #[error("malformed header line: {0:?}")]
    BadHeaderLine(String),
    #[error("body read error: {0}")]
    Body(String),
}

/// A parsed HTTP/1.1 request (method, path, headers, decoded body).
#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    /// Header values keyed by lower-case name.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// Case-insensitive header lookup.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_ascii_lowercase()).map(|s| s.as_str())
    }
}

/// An HTTP response to be serialised onto the wire.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    /// Extra headers (Content-Length is auto-added). Keys are sent as-is.
    pub headers: Vec<(String, String)>,
}

impl HttpResponse {
    pub fn new(status: u16) -> Self {
        Self { status, body: Vec::new(), headers: Vec::new() }
    }
}

// ---- RED stubs --------------------------------------------------------------

/// Parse an HTTP/1.1 request from raw bytes (headers + body). Handles both
/// `Content-Length` and `Transfer-Encoding: chunked` bodies.
pub fn parse_request(raw: &[u8]) -> Result<HttpRequest, HttpError> {
    let sep = find_subslice(raw, b"\r\n\r\n").ok_or(HttpError::NoHeaderTerminator)?;
    let head = std::str::from_utf8(&raw[..sep]).map_err(|_| HttpError::BadRequestLine)?;
    let body_start = sep + 4;
    let mut lines = head.split("\r\n");
    let req_line = lines.next().ok_or(HttpError::BadRequestLine)?;
    let mut parts = req_line.splitn(3, ' ');
    let method = parts.next().ok_or(HttpError::BadRequestLine)?.to_string();
    let path = parts.next().ok_or(HttpError::BadRequestLine)?.to_string();
    // version ignored (assumed 1.1)

    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (k, v) = line.split_once(':').ok_or_else(|| HttpError::BadHeaderLine(line.into()))?;
        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
    }

    let body = if let Some(te) = headers.get("transfer-encoding") {
        if te.to_ascii_lowercase().contains("chunked") {
            decode_chunked(&raw[body_start..]).map_err(|e| HttpError::Body(e.to_string()))?
        } else {
            Vec::new()
        }
    } else if let Some(cl) = headers.get("content-length") {
        let n: usize = cl.parse().map_err(|_| HttpError::Body(format!("bad Content-Length: {cl}")))?;
        if body_start + n > raw.len() {
            return Err(HttpError::Body("body shorter than Content-Length".into()));
        }
        raw[body_start..body_start + n].to_vec()
    } else {
        Vec::new()
    };

    Ok(HttpRequest { method, path, headers, body })
}

/// Format an [`HttpResponse`] as wire bytes (status line + headers + body).
pub fn format_response(resp: &HttpResponse) -> Vec<u8> {
    let reason = match resp.status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        406 => "Not Acceptable",
        100 => "Continue",
        _ => "OK",
    };
    let mut out = Vec::new();
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Length: {cl}\r\n",
        status = resp.status,
        cl = resp.body.len(),
    );
    out.extend_from_slice(head.as_bytes());
    for (k, v) in &resp.headers {
        out.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(&resp.body);
    out
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_request_with_content_length() {
        let raw = b"POST /Discover HTTP/1.1\r\n\
                    Host: [fe80::1]:8771\r\n\
                    Content-Type: application/x-apple-plist\r\n\
                    Content-Length: 5\r\n\
                    \r\n\
                    hello";
        let req = parse_request(raw).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/Discover");
        assert_eq!(req.header("content-type"), Some("application/x-apple-plist"));
        assert_eq!(req.body, b"hello");
    }

    #[test]
    fn parse_request_chunked_no_content_length() {
        // The iOS-18 quirk: chunked body, NO Content-Length header.
        let raw = b"POST /Ask HTTP/1.1\r\n\
                    Host: [fe80::1]:8771\r\n\
                    Transfer-Encoding: chunked\r\n\
                    \r\n\
                    5\r\nhello\r\n\
                    0\r\n\r\n";
        let req = parse_request(raw).unwrap();
        assert_eq!(req.path, "/Ask");
        assert!(req.header("content-length").is_none());
        assert_eq!(req.header("transfer-encoding"), Some("chunked"));
        assert_eq!(req.body, b"hello");
    }

    #[test]
    fn parse_request_get_with_empty_body() {
        let raw = b"GET / HTTP/1.1\r\nHost: example\r\n\r\n";
        let req = parse_request(raw).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/");
        assert!(req.body.is_empty());
    }

    #[test]
    fn parse_request_preserves_header_case_insensitive_lookup() {
        let raw = b"POST /Upload HTTP/1.1\r\n\
                    Content-Type: application/x-dvzip\r\n\
                    CONTENT-LENGTH: 3\r\n\
                    \r\n\
                    abc";
        let req = parse_request(raw).unwrap();
        assert_eq!(req.header("content-type"), Some("application/x-dvzip"));
        assert_eq!(req.header("CONTENT-TYPE"), Some("application/x-dvzip"));
    }

    #[test]
    fn format_response_basic() {
        let mut resp = HttpResponse::new(200);
        resp.headers.push(("Content-Type".into(), "text/plain".into()));
        resp.body = b"ok".to_vec();
        let bytes = format_response(&resp);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Type: text/plain\r\n"));
        assert!(text.contains("Content-Length: 2\r\n"));
        assert!(text.ends_with("\r\n\r\nok"));
    }

    #[test]
    fn format_response_no_body() {
        let resp = HttpResponse::new(404);
        let bytes = format_response(&resp);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.starts_with("HTTP/1.1 404 "));
        assert!(text.contains("Content-Length: 0\r\n"));
    }
}
