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
pub fn handle_one_connection(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    cfg: &ReceiverConfig,
) -> Result<HandleResult> {
    let raw = read_full_request(reader)?;
    let req = parse_request(&raw)?;
    let result = handle_post(&req, cfg);
    let response_bytes = format_response(&result.response);
    writer.write_all(&response_bytes)?;
    Ok(result)
}

/// Read a complete HTTP/1.1 request (headers + body) from a `BufRead`.
pub fn read_full_request(reader: &mut impl BufRead) -> Result<Vec<u8>> {
    // Read headers until \r\n\r\n
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

    // Determine how much more body to read.
    let head_str = std::str::from_utf8(&buf).unwrap_or("");
    let cl = parse_header_value(head_str, "content-length");
    let te = parse_header_value(head_str, "transfer-encoding");

    if let Some(cl_str) = cl {
        let len: usize = cl_str.parse().unwrap_or(0);
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body)?;
        buf.extend_from_slice(&body);
    } else if let Some(te_str) = te {
        if te_str.to_ascii_lowercase().contains("chunked") {
            buf.extend_from_slice(&read_chunked_stream(reader)?);
        }
    }
    Ok(buf)
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
    // Read the full request first, then write the response (avoids double-borrow of tls).
    let mut reader = BufReader::new(&mut tls);
    let raw = read_full_request(&mut reader)?;
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
        // Build a tiny dvzip payload.
        use crate::cpio::read_odc_cpio; // just to verify types compile
        let _ = read_odc_cpio;

        // Minimal CPIO with one file, then zlib-frame it.
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        use std::io::Write as IoWrite;
        let mut cpio = Vec::new();
        let name = "test.txt\0";
        let data = b"hello";
        let mut hdr = String::new();
        hdr.push_str("070707");
        hdr.push_str("000000"); // dev
        hdr.push_str("000001"); // ino
        hdr.push_str("100644"); // mode
        hdr.push_str("000000"); // uid
        hdr.push_str("000000"); // gid
        hdr.push_str("000001"); // nlink
        hdr.push_str("000000"); // rdev
        hdr.push_str("00000000000"); // mtime
        hdr.push_str(&format!("{:06o}", name.len())); // namesize
        hdr.push_str(&format!("{:011o}", data.len())); // filesize
        cpio.extend_from_slice(hdr.as_bytes());
        cpio.extend_from_slice(name.as_bytes());
        cpio.extend_from_slice(data);
        // trailer
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
        cpio.extend_from_slice(hdr.as_bytes());
        cpio.extend_from_slice(trailer.as_bytes());

        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&cpio).unwrap();
        let compressed = enc.finish().unwrap();
        let mut dvzip = Vec::new();
        dvzip.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        dvzip.extend_from_slice(&compressed);

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
}
