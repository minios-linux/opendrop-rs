//! AirDrop HTTPS server — the accept loop that ties TLS + HTTP parsing +
//! handlers together. The per-connection logic is extracted as a pure
//! function over `BufRead + Write` so it's unit-testable without sockets.
//!
//! Reference: `opendrop-patch/server.py.patched` (`start_server`, `do_POST`).

use crate::handlers::{HandleResult, handle_post};
use crate::http::{format_response, parse_request};
use crate::plist_impl::ReceiverConfig;
use anyhow::Result;
use rustls::ServerConfig;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tracing::{error, info, warn};

/// Handle a single HTTP request from `reader`, write the response to `writer`.
/// Pure (no sockets) — works with in-memory buffers for testing.
///
/// If the request carries `Expect: 100-continue`, an interim
/// `HTTP/1.1 100 Continue` line is written to `writer` *before* the body is
/// read — mirroring `opendrop-patch/server.py.patched::handle_upload`. Without
/// this, an Apple sender that waits for the 100 before shipping the body
/// deadlocks against a server blocked reading the body.
pub fn handle_one_connection(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    cfg: &ReceiverConfig,
) -> Result<HandleResult> {
    let raw = read_request_respecting_expect(reader, writer)?;
    let req = parse_request(&raw)?;
    let result = handle_post(&req, cfg);
    let response_bytes = format_response(&result.response);
    writer.write_all(&response_bytes)?;
    Ok(result)
}

/// Read a complete HTTP/1.1 request (headers + body) from a `BufRead`,
/// without handling `Expect: 100-continue`. Kept for callers/tests that don't
/// need the interim response.
pub fn read_full_request(reader: &mut impl BufRead) -> Result<Vec<u8>> {
    let header_bytes = read_headers(reader)?;
    let body = read_body(reader, &header_bytes)?;
    let mut full = header_bytes;
    full.extend_from_slice(&body);
    Ok(full)
}

/// Read a complete HTTP/1.1 request. If the headers carry
/// `Expect: 100-continue`, write `HTTP/1.1 100 Continue\r\n\r\n` to `writer`
/// **before** reading the body, so a peer waiting on the interim response
/// ships the body. This is the counterpart to the Python upload handler's
/// `send_response(100)` branch.
pub fn read_request_respecting_expect(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
) -> Result<Vec<u8>> {
    let header_bytes = read_headers(reader)?;
    let head_str = std::str::from_utf8(&header_bytes).unwrap_or("");
    if let Some(expect) = parse_header_value(head_str, "expect") {
        if expect.eq_ignore_ascii_case("100-continue") {
            writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        }
    }
    let body = read_body(reader, &header_bytes)?;
    let mut full = header_bytes;
    full.extend_from_slice(&body);
    Ok(full)
}

/// Read the HTTP/1.1 header section up to and including the `\r\n\r\n`
/// terminator. Returns the raw header bytes (including the terminator).
fn read_headers(reader: &mut impl BufRead) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    loop {
        let mut line = Vec::new();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&line);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    if buf.is_empty() {
        anyhow::bail!("connection closed before headers complete");
    }
    Ok(buf)
}

/// Read the body following already-parsed `header_bytes`. Dispatches on
/// `Content-Length` (exact read) or `Transfer-Encoding: chunked` (raw chunk
/// framing preserved for downstream `parse_request`).
fn read_body(reader: &mut impl BufRead, header_bytes: &[u8]) -> Result<Vec<u8>> {
    let head_str = std::str::from_utf8(header_bytes).unwrap_or("");
    if let Some(cl_str) = parse_header_value(head_str, "content-length") {
        let len: usize = cl_str.parse().unwrap_or(0);
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body)?;
        Ok(body)
    } else if let Some(te_str) = parse_header_value(head_str, "transfer-encoding") {
        if te_str.to_ascii_lowercase().contains("chunked") {
            Ok(read_chunked_stream(reader)?)
        } else {
            Ok(Vec::new())
        }
    } else {
        Ok(Vec::new())
    }
}

/// Serve forever on the given listener. Each connection is handled in a
/// thread. If `output_dir` is set, received files are saved there.
pub fn serve_forever(
    listener: TcpListener,
    tls_config: Arc<ServerConfig>,
    cfg: ReceiverConfig,
    output_dir: Option<PathBuf>,
) {
    let cfg = Arc::new(cfg);
    let output_dir = Arc::new(output_dir);
    for stream in listener.incoming() {
        match stream {
            Ok(tcp_stream) => {
                let tls_config = tls_config.clone();
                let cfg = cfg.clone();
                let output_dir = output_dir.clone();
                thread::spawn(move || {
                    if let Err(e) = handle_tls_connection(tcp_stream, tls_config, cfg, output_dir) {
                        warn!(error = %e, "connection error");
                    }
                });
            }
            Err(e) => error!(error = %e, "accept error"),
        }
    }
}

fn handle_tls_connection(
    tcp_stream: TcpStream,
    tls_config: Arc<ServerConfig>,
    cfg: Arc<ReceiverConfig>,
    output_dir: Arc<Option<PathBuf>>,
) -> Result<()> {
    let conn = rustls::ServerConnection::new(tls_config)?;
    let mut tls = rustls::StreamOwned::new(conn, tcp_stream);
    // Read headers, then body. The split lets us emit an interim
    // `HTTP/1.1 100 Continue` between them when the client sent
    // `Expect: 100-continue` — without that an Apple sender can deadlock
    // waiting for the 100 before shipping the body. We write through
    // `reader.get_mut()` (the underlying `&mut tls`) because `BufReader`
    // only buffers reads.
    let mut reader = BufReader::new(&mut tls);
    let header_bytes = read_headers(&mut reader)?;
    let head_str = std::str::from_utf8(&header_bytes).unwrap_or("");
    if let Some(expect) = parse_header_value(head_str, "expect") {
        if expect.eq_ignore_ascii_case("100-continue") {
            reader.get_mut().write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        }
    }
    let body = read_body(&mut reader, &header_bytes)?;
    let mut raw = header_bytes;
    raw.extend_from_slice(&body);

    let req = parse_request(&raw)?;
    let result = handle_post(&req, &cfg);
    let response_bytes = format_response(&result.response);
    tls.write_all(&response_bytes)?;
    for f in &result.received_files {
        info!(file = %f.name, bytes = f.bytes.len(), "received file");
        if let Some(dir) = output_dir.as_ref() {
            let path = dir.join(&f.name);
            match std::fs::write(&path, &f.bytes) {
                Ok(_) => info!(file = %f.name, path = %path.display(), "saved file"),
                Err(e) => warn!(file = %f.name, error = %e, "failed to save file"),
            }
        }
    }
    Ok(())
}

/// Bind a TCP listener on `[::]:port` (IPv6, dual-stack).
pub fn bind_listener(port: u16) -> Result<TcpListener> {
    let addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port);
    let listener = TcpListener::bind(addr)?;
    info!(port, "HTTPS listener bound");
    Ok(listener)
}

// ---- helpers ----

fn parse_header_value(headers: &str, name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    for line in headers.split("\r\n") {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().to_ascii_lowercase() == lower {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn read_chunked_stream(reader: &mut impl BufRead) -> Result<Vec<u8>> {
    let mut raw = Vec::new();
    loop {
        let mut size_line = String::new();
        reader.read_line(&mut size_line)?;
        raw.extend_from_slice(size_line.as_bytes());
        let size_str = size_line.trim();
        let size = usize::from_str_radix(size_str.split(';').next().unwrap().trim(), 16).unwrap_or(0);
        if size == 0 {
            // last-chunk: read trailing CRLF
            let mut trailer = String::new();
            reader.read_line(&mut trailer)?;
            raw.extend_from_slice(trailer.as_bytes());
            break;
        }
        let mut data = vec![0u8; size];
        reader.read_exact(&mut data)?;
        raw.extend_from_slice(&data);
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf)?;
        raw.extend_from_slice(&crlf);
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn cfg() -> ReceiverConfig {
        ReceiverConfig {
            computer_name: "luftlift-pc".into(),
            computer_model: "MacBookPro".into(),
            record_data: None,
        }
    }

    fn find_crlf_crlf(buf: &[u8]) -> usize {
        buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap()
    }

    #[test]
    fn handle_connection_discover_writes_plist_response() {
        let request = b"POST /Discover HTTP/1.1\r\n\
                        Host: localhost\r\n\
                        Content-Length: 0\r\n\
                        \r\n";
        let mut reader = Cursor::new(&request[..]);
        let mut writer = Vec::new();
        let result = handle_one_connection(&mut reader, &mut writer, &cfg()).unwrap();
        assert_eq!(result.response.status, 200);

        // Find header/body boundary in raw bytes (body is binary plist).
        let sep = find_crlf_crlf(&writer);
        let headers = std::str::from_utf8(&writer[..sep]).unwrap();
        assert!(headers.starts_with("HTTP/1.1 200 OK\r\n"));
        let body = &writer[sep + 4..];
        assert_eq!(&body[..8], b"bplist00");
    }

    #[test]
    fn handle_connection_chunked_ask_body() {
        // iOS-18 style: chunked body, no Content-Length.
        let request = b"POST /Ask HTTP/1.1\r\n\
                        Host: localhost\r\n\
                        Transfer-Encoding: chunked\r\n\
                        \r\n\
                        0\r\n\
                        \r\n";
        let mut reader = Cursor::new(&request[..]);
        let mut writer = Vec::new();
        let result = handle_one_connection(&mut reader, &mut writer, &cfg()).unwrap();
        assert_eq!(result.response.status, 200);
        let sep = find_crlf_crlf(&writer);
        assert_eq!(&writer[sep + 4..sep + 12], b"bplist00");
    }

    #[test]
    fn handle_connection_unknown_path_returns_404() {
        let request = b"POST /Unknown HTTP/1.1\r\nContent-Length: 0\r\n\r\n";
        let mut reader = Cursor::new(&request[..]);
        let mut writer = Vec::new();
        let result = handle_one_connection(&mut reader, &mut writer, &cfg()).unwrap();
        assert_eq!(result.response.status, 404);
        let headers = std::str::from_utf8(&writer).unwrap();
        assert!(headers.starts_with("HTTP/1.1 404"));
    }

    #[test]
    fn handle_connection_upload_dvzip_returns_files() {
        // Build a dvzip payload via the public encoders (one file "test.txt").
        use crate::{cpio::build_odc_cpio, dvzip::cpio_to_dvzip};
        let dvzip = cpio_to_dvzip(&build_odc_cpio(&[("test.txt", b"hello")]));

        let request = format!(
            "POST /Upload HTTP/1.1\r\n\
             Host: localhost\r\n\
             Content-Type: application/x-dvzip\r\n\
             Content-Length: {}\r\n\
             \r\n",
            dvzip.len()
        );
        let mut raw = request.into_bytes();
        raw.extend_from_slice(&dvzip);

        let mut reader = Cursor::new(&raw[..]);
        let mut writer = Vec::new();
        let result = handle_one_connection(&mut reader, &mut writer, &cfg()).unwrap();
        assert_eq!(result.response.status, 200);
        assert_eq!(result.received_files.len(), 1);
        assert_eq!(result.received_files[0].name, "test.txt");
        assert_eq!(result.received_files[0].bytes, b"hello");
    }

    #[test]
    fn handle_connection_sends_100_continue_for_expect() {
        // A sender that sends Expect: 100-continue must receive an interim
        // HTTP/1.1 100 Continue before the final response — otherwise the
        // sender deadlocks waiting for the 100 before shipping the body.
        // Mirrors opendrop-patch/server.py.patched::handle_upload's
        // `send_response(100)` branch.
        use crate::{cpio::build_odc_cpio, dvzip::cpio_to_dvzip};
        let dvzip = cpio_to_dvzip(&build_odc_cpio(&[("note.txt", b"expect me")]));
        let request = format!(
            "POST /Upload HTTP/1.1\r\n\
             Host: localhost\r\n\
             Content-Type: application/x-dvzip\r\n\
             Content-Length: {}\r\n\
             Expect: 100-continue\r\n\
             \r\n",
            dvzip.len()
        );
        let mut raw = request.into_bytes();
        raw.extend_from_slice(&dvzip);

        let mut reader = Cursor::new(&raw[..]);
        let mut writer = Vec::new();
        let result = handle_one_connection(&mut reader, &mut writer, &cfg()).unwrap();
        assert_eq!(result.response.status, 200);
        assert_eq!(result.received_files[0].bytes, b"expect me");

        // The writer must contain the interim 100 Continue BEFORE the final
        // 200 response. The body was still extracted, proving the body read
        // happened after the 100 was emitted.
        let out = String::from_utf8_lossy(&writer);
        let cont_idx = out
            .find("HTTP/1.1 100 Continue")
            .expect("interim 100 Continue must be sent");
        let final_idx = out.find("HTTP/1.1 200").expect("final 200 response must follow");
        assert!(cont_idx < final_idx, "100 Continue must precede the final response");
    }

    #[test]
    fn handle_connection_no_100_continue_without_expect_header() {
        // Without Expect: 100-continue there must be no interim response.
        let request = b"POST /Discover HTTP/1.1\r\n\
                         Host: localhost\r\n\
                         Content-Length: 0\r\n\
                         \r\n";
        let mut reader = Cursor::new(&request[..]);
        let mut writer = Vec::new();
        let _ = handle_one_connection(&mut reader, &mut writer, &cfg()).unwrap();
        let out = String::from_utf8_lossy(&writer);
        assert!(!out.contains("100 Continue"), "no 100 Continue without Expect header");
        assert!(out.starts_with("HTTP/1.1 200"));
    }
}
