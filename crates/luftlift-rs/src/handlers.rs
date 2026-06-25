//! AirDrop protocol handlers — pure functions that route a parsed
//! [`crate::http::HttpRequest`] to the right action (`/Discover`, `/Ask`,
//! `/Upload`) and produce an [`crate::http::HttpResponse`] plus any received
//! files. I/O-free for easy testing. See `opendrop-patch/server.py.patched`
//! (`do_POST`, `handle_*`).

use crate::cpio::read_odc_cpio;
use crate::dvzip::dvzip_to_cpio;
use crate::http::{HttpRequest, HttpResponse};
use crate::plist_impl::{ReceiverConfig, build_ask_response, build_discover_response, parse_ask_request};
use tracing::{info, warn};

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
    match req.path.as_str() {
        "/Discover" => handle_discover(req, cfg),
        "/Ask" => handle_ask(req, cfg),
        "/Upload" => handle_upload(req),
        path => {
            warn!(path, "unknown POST path");
            HandleResult { response: HttpResponse::new(404), received_files: Vec::new() }
        }
    }
}

/// Decode an Upload body (dvzip or raw CPIO) into extracted file entries.
/// Returns the list of (name, bytes) found in the archive.
pub fn decode_upload_body(content_type: &str, body: &[u8]) -> Vec<ReceivedFile> {
    let ct = content_type.to_ascii_lowercase();
    let cpio_bytes: Vec<u8> = if ct == "application/x-dvzip" {
        dvzip_to_cpio(body)
    } else {
        body.to_vec()
    };
    read_odc_cpio(&cpio_bytes)
        .into_iter()
        .map(|e| ReceivedFile { name: e.name, bytes: e.data.to_vec() })
        .collect()
}

fn handle_discover(_req: &HttpRequest, cfg: &ReceiverConfig) -> HandleResult {
    info!("Discover request — appearing in sender picker");
    let mut resp = HttpResponse::new(200);
    resp.headers.push(("Content-Type".into(), "application/x-apple-plist".into()));
    resp.body = build_discover_response(cfg);
    HandleResult { response: resp, received_files: Vec::new() }
}

fn handle_ask(req: &HttpRequest, cfg: &ReceiverConfig) -> HandleResult {
    let ask = parse_ask_request(&req.body);
    match &ask {
        Ok(a) => info!(
            transfer_id = %a.transfer_id,
            transfer_type = %a.transfer_type,
            sender = %a.sender_computer_name,
            file_count = a.files.len(),
            "Ask request — auto-accepting"
        ),
        Err(e) => warn!(error = %e, "failed to parse Ask body"),
    }
    let mut resp = HttpResponse::new(200);
    resp.headers.push(("Content-Type".into(), "application/x-apple-plist".into()));
    resp.body = build_ask_response(cfg);
    HandleResult { response: resp, received_files: Vec::new() }
}

fn handle_upload(req: &HttpRequest) -> HandleResult {
    let ctype = req.header("content-type").unwrap_or("");
    if !ctype.eq_ignore_ascii_case("application/x-dvzip")
        && !ctype.eq_ignore_ascii_case("application/x-cpio")
    {
        warn!(content_type = ctype, "unsupported Upload content-type");
        let mut resp = HttpResponse::new(406);
        resp.headers.push(("Connection".into(), "close".into()));
        return HandleResult { response: resp, received_files: Vec::new() };
    }

    let files = decode_upload_body(ctype, &req.body);
    for f in &files {
        info!(file = %f.name, bytes = f.bytes.len(), "received file");
    }
    info!(count = files.len(), "Upload complete");

    let mut resp = HttpResponse::new(200);
    resp.headers.push(("Connection".into(), "close".into()));
    HandleResult { response: resp, received_files: files }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpRequest;
    use crate::plist_impl::ReceiverConfig;
    use crate::{cpio::build_odc_cpio, dvzip::cpio_to_dvzip};
    use plist::{Dictionary, Value};
    use std::collections::HashMap;

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

    // ---- /Upload decode helpers (reuse the public encoders) ----

    fn make_dvzip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        cpio_to_dvzip(&build_odc_cpio(entries))
    }

    #[test]
    fn decode_upload_body_dvzip_extracts_files() {
        let dvzip = make_dvzip(&[("photo.jpg", &[0xFF, 0xD8, 0xFF, 0xE0])]);
        let files = decode_upload_body("application/x-dvzip", &dvzip);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "photo.jpg");
        assert_eq!(files[0].bytes, vec![0xFF, 0xD8, 0xFF, 0xE0]);
    }

    #[test]
    fn decode_upload_body_cpio_extracts_files() {
        let cpio = build_odc_cpio(&[("hello.txt", b"world"), ("a.bin", &[0, 1, 2])]);
        let files = decode_upload_body("application/x-cpio", &cpio);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, "hello.txt");
        assert_eq!(files[0].bytes, b"world");
        assert_eq!(files[1].name, "a.bin");
    }

    // ---- /Upload full handler ----

    #[test]
    fn handle_upload_dvzip_returns_200_and_received_files() {
        let dvzip = make_dvzip(&[("doc.txt", b"hi")]);
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
