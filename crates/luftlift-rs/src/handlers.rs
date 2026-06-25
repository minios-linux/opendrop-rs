//! AirDrop protocol handlers — pure functions that route a parsed
//! [`crate::http::HttpRequest`] to the right action (`/Discover`, `/Ask`,
//! `/Upload`) and produce an [`crate::http::HttpResponse`] plus any received
//! files. I/O-free for easy testing. See `opendrop-patch/server.py.patched`
//! (`do_POST`, `handle_*`).

use crate::cpio::read_odc_cpio;
use crate::dvzip::dvzip_to_cpio;
use crate::http::{HttpRequest, HttpResponse};
use crate::plist_impl::{ReceiverConfig, build_ask_response, build_discover_response, parse_ask_request};
use tracing::{debug, info, warn};

/// A file extracted from an `/Upload` body.
#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The outcome of handling a request: the response to send, plus any files
/// received (empty for non-Upload routes).
#[derive(Debug)]
pub struct HandleResult {
    pub response: HttpResponse,
    pub received_files: Vec<ReceivedFile>,
}

/// Route a POST request to the matching AirDrop handler. Returns a 404
/// [`HttpResponse`] for unknown paths.
pub fn handle_post(req: &HttpRequest, cfg: &ReceiverConfig) -> HandleResult {
    let _ = (req, cfg);
    HandleResult { response: HttpResponse::new(404), received_files: Vec::new() }
}

// ---- Upload body decoder (pure, reused by tests + handler) -----

/// Decode an Upload body (dvzip or raw CPIO) into extracted file entries.
/// Returns the list of (name, bytes) found in the archive.
pub fn decode_upload_body(content_type: &str, body: &[u8]) -> Vec<ReceivedFile> {
    let _ = (content_type, body);
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpRequest;
    use crate::plist_impl::ReceiverConfig;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use plist::{Dictionary, Value};
    use std::collections::HashMap;
    use std::io::Write;

    fn cfg() -> ReceiverConfig {
        ReceiverConfig {
            computer_name: "luftlift-pc".into(),
            computer_model: "MacBookPro".into(),
            record_data: None,
        }
    }

    fn post(path: &str, headers: Vec<(&str, &str)>, body: Vec<u8>) -> HttpRequest {
        let mut h = HashMap::new();
        for (k, v) in headers {
            h.insert(k.to_ascii_lowercase(), v.to_string());
        }
        HttpRequest { method: "POST".into(), path: path.into(), headers: h, body }
    }

    // ---- /Discover ----

    #[test]
    fn handle_discover_returns_200_with_receiver_plist() {
        let req = post("/Discover", vec![("Content-Type", "application/x-apple-plist")], Vec::new());
        let res = handle_post(&req, &cfg());
        assert_eq!(res.response.status, 200);
        assert!(!res.response.body.is_empty());
        assert_eq!(&res.response.body[..8], b"bplist00");
        let val = Value::from_reader(std::io::Cursor::new(&res.response.body[..])).unwrap();
        assert_eq!(
            val.as_dictionary().unwrap().get("ReceiverComputerName").unwrap().as_string(),
            Some("luftlift-pc"),
        );
    }

    // ---- /Ask ----

    #[test]
    fn handle_ask_returns_200_and_logs_transfer() {
        // Build a minimal Ask request body.
        let mut d = Dictionary::new();
        d.insert("TransferID".into(), Value::String("XYZ-123".into()));
        d.insert("TransferType".into(), Value::String("links".into()));
        d.insert("SenderComputerName".into(), Value::String("iPhone".into()));
        d.insert("SenderModelName".into(), Value::String("iPhone".into()));
        d.insert("BundleID".into(), Value::String("com.apple.mobilesafari".into()));
        let mut body = Vec::new();
        Value::Dictionary(d).to_writer_binary(&mut body).unwrap();

        let req = post("/Ask", vec![("Content-Type", "application/x-apple-plist")], body);
        let res = handle_post(&req, &cfg());
        assert_eq!(res.response.status, 200);
        let val = Value::from_reader(std::io::Cursor::new(&res.response.body[..])).unwrap();
        assert_eq!(
            val.as_dictionary().unwrap().get("ReceiverComputerName").unwrap().as_string(),
            Some("luftlift-pc"),
        );
    }

    // ---- /Upload decode helpers ----

    fn zlib_encode(data: &[u8]) -> Vec<u8> {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    fn build_cpio(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, &(name, data)) in entries.iter().enumerate() {
            let name_nul = format!("{name}\0");
            let mut hdr = String::new();
            hdr.push_str("070707");
            hdr.push_str("000000"); // dev
            hdr.push_str(&format!("{:06o}", i + 1)); // ino
            hdr.push_str("100644"); // mode
            hdr.push_str("000000"); // uid
            hdr.push_str("000000"); // gid
            hdr.push_str("000001"); // nlink
            hdr.push_str("000000"); // rdev
            hdr.push_str("00000000000"); // mtime
            hdr.push_str(&format!("{:06o}", name_nul.len())); // namesize
            hdr.push_str(&format!("{:011o}", data.len())); // filesize
            out.extend_from_slice(hdr.as_bytes());
            out.extend_from_slice(name_nul.as_bytes());
            out.extend_from_slice(data);
        }
        let trailer = "TRAILER!!!\0";
        let mut hdr = String::new();
        hdr.push_str("070707");
        hdr.push_str("000000");
        hdr.push_str("000000");
        hdr.push_str("100644");
        hdr.push_str("000000");
        hdr.push_str("000000");
        hdr.push_str("000001");
        hdr.push_str("000000");
        hdr.push_str("00000000000");
        hdr.push_str(&format!("{:06o}", trailer.len()));
        hdr.push_str("00000000000");
        out.extend_from_slice(hdr.as_bytes());
        out.extend_from_slice(trailer.as_bytes());
        out
    }

    fn frame_dvzip(cpio: &[u8]) -> Vec<u8> {
        let compressed = zlib_encode(cpio);
        let mut out = Vec::new();
        out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        out.extend_from_slice(&compressed);
        out
    }

    #[test]
    fn decode_upload_body_dvzip_extracts_files() {
        let cpio = build_cpio(&[("photo.jpg", &[0xFF, 0xD8, 0xFF, 0xE0])]);
        let dvzip = frame_dvzip(&cpio);
        let files = decode_upload_body("application/x-dvzip", &dvzip);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "photo.jpg");
        assert_eq!(files[0].bytes, vec![0xFF, 0xD8, 0xFF, 0xE0]);
    }

    #[test]
    fn decode_upload_body_cpio_extracts_files() {
        let cpio = build_cpio(&[("hello.txt", b"world"), ("a.bin", &[0, 1, 2])]);
        let files = decode_upload_body("application/x-cpio", &cpio);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, "hello.txt");
        assert_eq!(files[0].bytes, b"world");
        assert_eq!(files[1].name, "a.bin");
    }

    // ---- /Upload full handler ----

    #[test]
    fn handle_upload_dvzip_returns_200_and_received_files() {
        let cpio = build_cpio(&[("doc.txt", b"hi")]);
        let dvzip = frame_dvzip(&cpio);
        let req = post(
            "/Upload",
            vec![("Content-Type", "application/x-dvzip"), ("Transfer-Encoding", "chunked")],
            dvzip,
        );
        let res = handle_post(&req, &cfg());
        assert_eq!(res.response.status, 200);
        assert_eq!(res.received_files.len(), 1);
        assert_eq!(res.received_files[0].name, "doc.txt");
        assert_eq!(res.received_files[0].bytes, b"hi");
    }

    // ---- unknown path ----

    #[test]
    fn handle_unknown_path_returns_404() {
        let req = post("/Foo", vec![], Vec::new());
        let res = handle_post(&req, &cfg());
        assert_eq!(res.response.status, 404);
    }
}
