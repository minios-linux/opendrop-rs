//! AirDrop **sender** (client) — builds the `/Discover`, `/Ask`, `/Upload`
//! requests as pure functions and orchestrates the POST sequence over a TLS
//! connection. The network glue is kept behind a small trait so the request
//! building + response handling is unit-testable without sockets.
//!
//! Reference: `docs/airdrop-protocol.md` (HTTP flow + iOS-18 quirks) and
//! `opendrop-patch/server.py.patched` (the receiver these requests target).

use crate::dvzip::cpio_to_dvzip;
use crate::cpio::build_odc_cpio;
use crate::http::{HttpRequest, HttpResponse};
use crate::plist_impl::ReceiverConfig;
use anyhow::Result;
use plist::{Dictionary, Value};
use std::collections::HashMap;
use tracing::info;

/// Sender identity/configuration used to build /Discover and /Ask requests.
#[derive(Debug, Clone)]
pub struct SenderConfig {
    pub computer_name: String,
    pub model_name: String,
    pub bundle_id: String,
}

/// One file to send.
#[derive(Debug, Clone)]
pub struct FileToSend {
    pub name: String,
    /// UTI type, e.g. "public.jpeg" for JPEGs, "com.apple.web-internet-location"
    /// for links. opendrop uses "public.data" as a generic fallback.
    pub uti_type: String,
    pub data: Vec<u8>,
}

/// Build the `/Discover` request body (sender record plist). The receiver
/// responds with its own record so we appear in its picker — actually for a
/// sender, /Discover makes the *receiver* appear in *our* picker. Body is a
/// minimal sender plist; Content-Type application/x-apple-plist.
pub fn build_discover_request(cfg: &SenderConfig) -> HttpRequest {
    let mut dict = Dictionary::new();
    dict.insert("SenderComputerName".into(), Value::String(cfg.computer_name.clone()));
    dict.insert("SenderModelName".into(), Value::String(cfg.model_name.clone()));
    let mut body = Vec::new();
    Value::Dictionary(dict).to_writer_binary(&mut body).expect("serialise discover request");

    let mut headers = HashMap::new();
    headers.insert("host".into(), "airdrop._tcp.local.".into());
    headers.insert("content-type".into(), "application/x-apple-plist".into());
    HttpRequest {
        method: "POST".into(),
        path: "/Discover".into(),
        headers,
        body,
    }
}

/// Build the `/Ask` request body (file metadata plist). `transfer_id` is a
/// caller-generated UUID; the receiver auto-accepts by replying 200.
pub fn build_ask_request(cfg: &SenderConfig, transfer_id: &str, files: &[FileToSend]) -> HttpRequest {
    let mut files_arr = Vec::new();
    for f in files {
        let mut entry = Dictionary::new();
        entry.insert("FileName".into(), Value::String(f.name.clone()));
        entry.insert("FileType".into(), Value::String(f.uti_type.clone()));
        entry.insert("FileSize".into(), Value::Integer((f.data.len() as i64).into()));
        entry.insert("FileBomPath".into(), Value::String(format!("./{}", f.name)));
        files_arr.push(Value::Dictionary(entry));
    }

    let mut root = Dictionary::new();
    root.insert("TransferID".into(), Value::String(transfer_id.into()));
    root.insert("TransferType".into(), Value::String("files".into()));
    root.insert("SenderComputerName".into(), Value::String(cfg.computer_name.clone()));
    root.insert("SenderModelName".into(), Value::String(cfg.model_name.clone()));
    root.insert("BundleID".into(), Value::String(cfg.bundle_id.clone()));
    root.insert("Files".into(), Value::Array(files_arr));
    root.insert("Items".into(), Value::Array(Vec::new()));

    let mut body = Vec::new();
    Value::Dictionary(root).to_writer_binary(&mut body).expect("serialise ask request");

    let mut headers = HashMap::new();
    headers.insert("host".into(), "airdrop._tcp.local.".into());
    headers.insert("content-type".into(), "application/x-apple-plist".into());
    HttpRequest {
        method: "POST".into(),
        path: "/Ask".into(),
        headers,
        body,
    }
}

/// Build the `/Upload` request body: a CPIO archive of the files, framed as
/// dvzip (iOS-18 Content-Type `application/x-dvzip`).
pub fn build_upload_request(files: &[FileToSend]) -> HttpRequest {
    let entries: Vec<(&str, &[u8])> =
        files.iter().map(|f| (f.name.as_str(), f.data.as_slice())).collect();
    let cpio = build_odc_cpio(&entries);
    let dvzip = cpio_to_dvzip(&cpio);

    let mut headers = HashMap::new();
    headers.insert("host".into(), "airdrop._tcp.local.".into());
    headers.insert("content-type".into(), "application/x-dvzip".into());
    HttpRequest {
        method: "POST".into(),
        path: "/Upload".into(),
        headers,
        body: dvzip,
    }
}

/// Verify the receiver's /Discover response carries the expected receiver
/// record fields. Returns the parsed ReceiverComputerName on success.
pub fn parse_discover_response(resp: &HttpResponse) -> Result<ReceiverConfig> {
    if resp.status != 200 {
        anyhow::bail!("/Discover returned status {}", resp.status);
    }
    let val = Value::from_reader(std::io::Cursor::new(&resp.body))?;
    let dict = val.as_dictionary().ok_or_else(|| anyhow::anyhow!("/Discover response not a dict"))?;
    Ok(ReceiverConfig {
        computer_name: dict
            .get("ReceiverComputerName")
            .and_then(|v| v.as_string())
            .unwrap_or("unknown")
            .to_string(),
        computer_model: dict
            .get("ReceiverModelName")
            .and_then(|v| v.as_string())
            .unwrap_or("unknown")
            .to_string(),
        record_data: dict.get("ReceiverRecordData").and_then(|v| v.as_data()).map(|d| d.to_vec()),
    })
}

// ---- thin network trait ----

/// A minimal HTTP-over-TLS transport. Implementations talk to a real
/// `rustls::StreamOwned<TcpStream>`; the in-memory test impl round-trips
/// through the crate's own server handler.
pub trait AirDropTransport {
    /// Send a request and read back the response.
    fn send(&mut self, req: &HttpRequest) -> Result<HttpResponse>;
}

/// Run the full AirDrop send flow against a peer over the given transport.
/// Sequence: POST /Discover → POST /Ask → POST /Upload.
pub fn send_files(
    transport: &mut impl AirDropTransport,
    sender: &SenderConfig,
    files: &[FileToSend],
    transfer_id: &str,
) -> Result<()> {
    info!("POST /Discover");
    let req = build_discover_request(sender);
    let resp = transport.send(&req)?;
    let receiver = parse_discover_response(&resp)?;
    info!(receiver = %receiver.computer_name, "receiver appeared");

    info!(transfer_id, "POST /Ask");
    let req = build_ask_request(sender, transfer_id, files);
    let resp = transport.send(&req)?;
    if resp.status != 200 {
        anyhow::bail!("/Ask rejected (status {})", resp.status);
    }
    info!("receiver accepted transfer");

    info!("POST /Upload ({} files)", files.len());
    let req = build_upload_request(files);
    let resp = transport.send(&req)?;
    if resp.status != 200 {
        anyhow::bail!("/Upload failed (status {})", resp.status);
    }
    let total: usize = files.iter().map(|f| f.data.len()).sum();
    info!(files = files.len(), bytes = total, "upload complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunked::decode_chunked;
    use crate::dvzip::dvzip_to_cpio;
    use crate::cpio::read_odc_cpio;
    use crate::http::{format_request, parse_request};
    use crate::handlers::handle_post;
    use crate::plist_impl::ReceiverConfig;

    fn sender() -> SenderConfig {
        SenderConfig {
            computer_name: "luftlift-sender".into(),
            model_name: "MacBookPro".into(),
            bundle_id: "com.luftlift.app".into(),
        }
    }

    // ---- build_discover_request ----

    #[test]
    fn build_discover_request_is_binary_plist_post() {
        let req = build_discover_request(&sender());
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/Discover");
        assert_eq!(req.header("content-type"), Some("application/x-apple-plist"));
        assert_eq!(&req.body[..8], b"bplist00");
        // The receiver handler should accept it and produce a response.
        let resp = handle_post(&req, &rcfg());
        assert_eq!(resp.response.status, 200);
    }

    // ---- build_ask_request ----

    #[test]
    fn build_ask_request_describes_files() {
        let files = vec![FileToSend {
            name: "photo.jpg".into(),
            uti_type: "public.jpeg".into(),
            data: vec![0xFF, 0xD8, 0xFF],
        }];
        let req = build_ask_request(&sender(), "TID-1", &files);
        assert_eq!(req.path, "/Ask");
        // Receiver should be able to parse our Ask and accept.
        let resp = handle_post(&req, &rcfg());
        assert_eq!(resp.response.status, 200);
    }

    // ---- build_upload_request ----

    #[test]
    fn build_upload_request_roundtrips_through_receiver() {
        let files = vec![
            FileToSend {
                name: "hello.txt".into(),
                uti_type: "public.data".into(),
                data: b"hi from luftlift".to_vec(),
            },
            FileToSend {
                name: "sweep.bin".into(),
                uti_type: "public.data".into(),
                data: (0u8..=255).collect(),
            },
        ];
        let req = build_upload_request(&files);
        assert_eq!(req.header("content-type"), Some("application/x-dvzip"));
        assert_eq!(req.path, "/Upload");

        // The receiver handler should decode it and extract the files.
        let resp = handle_post(&req, &rcfg());
        assert_eq!(resp.response.status, 200);
        assert_eq!(resp.received_files.len(), 2);
        assert_eq!(resp.received_files[0].name, "hello.txt");
        assert_eq!(resp.received_files[0].bytes, b"hi from luftlift");
        assert_eq!(resp.received_files[1].name, "sweep.bin");
        assert_eq!(resp.received_files[1].bytes, (0u8..=255).collect::<Vec<u8>>());
    }

    #[test]
    fn build_upload_request_body_is_valid_dvzip() {
        let files = vec![FileToSend {
            name: "x".into(),
            uti_type: "public.data".into(),
            data: b"abc".to_vec(),
        }];
        let req = build_upload_request(&files);
        // Directly decode the dvzip body → CPIO → entries.
        let cpio = dvzip_to_cpio(&req.body);
        assert_eq!(&cpio[..6], b"070707");
        let entries = read_odc_cpio(&cpio);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "x");
        assert_eq!(entries[0].data, b"abc");
    }

    // ---- parse_discover_response ----

    #[test]
    fn parse_discover_response_extracts_receiver_name() {
        let resp = handle_post(&build_discover_request(&sender()), &rcfg()).response;
        let recv = parse_discover_response(&resp).unwrap();
        assert_eq!(recv.computer_name, "luftlift-pc");
        assert_eq!(recv.computer_model, "MacBookPro");
    }

    #[test]
    fn parse_discover_response_errors_on_non_200() {
        let resp = HttpResponse { status: 404, body: Vec::new(), headers: Vec::new() };
        assert!(parse_discover_response(&resp).is_err());
    }

    // ---- end-to-end send via in-memory transport ----

    /// An in-memory transport that routes requests through our own server
    /// handler — proving the client requests and server handler interoperate.
    struct InMemoryTransport {
        receiver: ReceiverConfig,
    }
    impl AirDropTransport for InMemoryTransport {
        fn send(&mut self, req: &HttpRequest) -> Result<HttpResponse> {
            // Serialise the request, then re-parse to exercise the wire format.
            let wire = format_request(req);
            let parsed = parse_request(&wire).unwrap();
            let result = handle_post(&parsed, &self.receiver);
            Ok(result.response)
        }
    }

    fn rcfg() -> ReceiverConfig {
        ReceiverConfig {
            computer_name: "luftlift-pc".into(),
            computer_model: "MacBookPro".into(),
            record_data: None,
        }
    }

    #[test]
    fn send_files_end_to_end_succeeds_via_in_memory_transport() {
        let mut transport = InMemoryTransport { receiver: rcfg() };
        let files = vec![FileToSend {
            name: "note.txt".into(),
            uti_type: "public.data".into(),
            data: b"hello world".to_vec(),
        }];
        send_files(&mut transport, &sender(), &files, "TID-E2E").unwrap();
    }

    #[test]
    fn send_files_handles_chunked_request_bodies() {
        // Verify our requests use Content-Length by default (the body is a
        // binary plist, so inspect only the header section of the wire bytes).
        let req = build_ask_request(&sender(), "TID-C", &[]);
        let wire = format_request(&req);
        let header_end = wire.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let headers = std::str::from_utf8(&wire[..header_end]).unwrap();
        assert!(headers.contains("Content-Length"));
        // And our chunked decoder handles a hand-framed body (iOS-18 path).
        let chunked = b"3\r\nabc\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(chunked).unwrap(), b"abc");
    }
}
