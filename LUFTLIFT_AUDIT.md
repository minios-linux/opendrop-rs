# LUFTLIFT_AUDIT.md — opendrop (Python) vs luftlift (Rust) layer-by-layer

Goal: make luftlift byte-for-byte faithful to real AirDrop so it interoperates
with Apple devices. Each section lists what the reference does, what luftlift
does today, the gap, severity, and the fix. A RESULT note is appended per layer
as fixes land.

Reference anchors:
- Python: `opendrop-patch/server.py.patched` (`AirDropServerHandler`), `opendrop-patch/dvzip.py`
- Protocol notes: `docs/airdrop-protocol.md`
- luftlift: `crates/luftlift-rs/src/{dvzip,cpio,chunked,http,handlers,plist_impl,mdns,tls,server,client}.rs`

Baseline before this audit: `cargo test -p luftlift-rs` = 66 passed;
`cargo clippy -p luftlift-rs --all-targets -- -D warnings` clean.

---

## 1. dvzip framing — OK (minor behavioural divergence on error)

**Reference** (`dvzip.py::dvzip_to_cpio`): a pure loop reading `[u32 BE length]
[zlib stream]` blocks, inflating each via `zlib.decompress` and concatenating.
Stops cleanly on a zero-length block or a trailing short frame (guard:
`block_len == 0 or pos + block_len > len(data)`). If `zlib.decompress` raises
on a malformed block, the **whole decode fails** (exception propagates).

**luftlift** (`dvzip.rs::dvzip_to_cpio`): byte-identical loop, same BE-u32
length prefix, same stop guard, same inflate+concat. Verified by
`dvzip_decodes_single_framed_zlib_block_to_cpio`,
`dvzip_decodes_multiple_framed_zlib_blocks_to_cpio`, and
`cpio_to_dvzip_produces_valid_framing` (asserts the zlib header `0x78 0x9c`).
On a malformed zlib block the Rust decoder logs at `debug!` and **skips** it
rather than failing the whole decode (defensive — better partial recovery out
of a truncated capture).

**Gap / severity:** NONE for valid inputs (the on-wire shape is faithful and
the real `/tmp/upload.bin` fixture documented in the PORT.md round-trips).
LOW on error handling: Rust is more lenient than Python. Intentional and
arguably better for a receiver that should not crash on a single bad block.

**RESULT:** No change needed. Behavioural divergence is intentional and
documented in the rustdoc.

---

## 2. ODC CPIO — OK (encode + decode faithful)

**Reference** (`docs/airdrop-protocol.md` + `dvzip.py`): the inner format is
POSIX ODC CPIO (old portable ASCII, magic `070707`):
```
offset  len  field
0       6    c_magic     "070707"
6       6    c_dev
12      6    c_ino
18      6    c_mode
24      6    c_uid
30      6    c_gid
36      6    c_nlink
42      6    c_rdev
48      11   c_mtime
59      6    c_namesize  (includes trailing NUL)
65      11   c_filesize
```
76-byte header, then `namesize` name bytes (NUL-terminated), then `filesize`
data bytes. **No alignment padding** in ODC (unlike newc `070701`). Archive
terminated by an entry named `TRAILER!!!` with a zero-length body.

**luftlift** (`cpio.rs`): `read_odc_cpio` walks the 76-byte header with the
exact field offsets above, strips the trailing NUL from the name, stops at
`TRAILER!!!` (and suppresses it from the emitted entries), stops cleanly on a
short/malformed trailing frame. `build_odc_cpio` is the inverse encoder the
sender uses; emits `070707` magic, `100644` mode (regular file 0644), and a
proper `TRAILER!!!\0` terminator. Round-trip verified by
`build_odc_cpio_roundtrips_through_read_odc_cpio`,
`read_odc_cpio_preserves_exact_binary_bytes` (byte sweep 0..=255 incl.
0x00/0xFF/0x0A), and `read_odc_cpio_stops_at_trailer_and_omits_it`.

**Gap / severity:** NONE. Decoder matches the format spec; encoder round-trips
through the decoder and produces the expected header/name/trailer layout.

**RESULT:** No change needed.

---

## 3. Chunked HTTP — GAP (Expect: 100-continue not handled) — FIXED

**Reference** (`server.py.patched::_read_request_body`): reads either a
`Content-Length` body or a chunked body (the iOS-18 quirk: `/Discover` and
`/Ask` arrive with `Transfer-Encoding: chunked` and **no** `Content-Length`).
Chunked parsing: loop reading hex size lines, `int(line, 16)`, read `size`
bytes, consume CRLF; stop at size 0; consume trailing CRLF.

**luftlift** (`chunked.rs::decode_chunked`, `http.rs::parse_request`,
`server.rs::read_chunked_stream`): three layers of chunked handling, all
verified — pure decoder `decode_chunked` handles RFC 7230 §4.1 examples,
chunk extensions (`;foo=bar`), trailers, binary bytes, and rejects bad hex /
truncated data. `parse_request` picks chunked vs Content-Length automatically.
`read_chunked_stream` handles the streaming case on a live `BufRead`.

**Gap / severity:** HIGH for **`Expect: 100-continue`**. The reference
`handle_upload` explicitly handles it:
```python
if self.headers.get("expect", "").lower() == "100-continue":
    self.send_response(100)
    self.send_header("Content-Length", 0)
    self.end_headers()
```
luftlift's `handle_tls_connection` calls `read_full_request` which reads
**headers + body in one shot** — so if an Apple sender sends `Expect:
100-continue` and waits for the 100 response before shipping the body, the
server blocks reading a body the peer will never send until timeout →
**deadlock**. The fix is to split the read into headers-first → maybe-send-100
→ read-body.

**RESULT:** Fixed. `server.rs` now reads headers first, emits an
interim `HTTP/1.1 100 Continue\r\n\r\n` when `Expect: 100-continue` is
present, then reads the body. Covered by `handle_tls_connection_sends_100_continue_for_expect`.

---

## 4. plist Discover / Ask / Upload — MINOR (JSON whitespace, SenderID)

**Reference** (`server.py.patched::handle_discover`, `handle_ask`,
`docs/airdrop-protocol.md`):
- **Discover response** keys: `ReceiverMediaCapabilities` (raw bytes = a JSON
  blob produced by `json.dumps({"Version": 1})` → `{"Version": 1}` WITH a
  space after the colon), `ReceiverComputerName`, `ReceiverModelName`, and
  optionally `ReceiverRecordData`.
- **Ask response** keys: `ReceiverModelName`, `ReceiverComputerName`.
- **Ask request** keys (from a real iOS-18.6.2 Safari link capture):
  `TransferID`, `TransferType`, `SenderID` (hex bytes), `SenderComputerName`,
  `SenderModelName`, `BundleID`, `Files: [{FileName,FileType,FileSize,
  FileBomPath}]`, `Items`, `FileIcon`, `SenderRecordData`.
- **Upload request**: an archive body (no plist), Content-Type `x-dvzip` /
  `x-cpio`.

**luftlift** (`plist_impl.rs`, `client.rs`):
- `build_discover_response` emits all three/four keys as a binary plist
  (`bplist00`). Verified by `discover_response_is_valid_binary_plist_with_required_fields`.
- `build_ask_response` emits `ReceiverModelName` + `ReceiverComputerName`.
- `parse_ask_request` pulls every Ask-request field, all treated as optional
  so a missing one never rejects a genuine transfer (real iOS sends do not
  always include every key).
- Sender side (`build_ask_request`) emits `TransferID`, `TransferType`,
  `SenderComputerName`, `SenderModelName`, `BundleID`, `Files`, `Items` — but
  **omits `SenderID`** that the real capture includes.
- `ReceiverMediaCapabilities` hardcodes `{"Version":1}` (no space), diverging
  from Python's `json.dumps` default of `{"Version": 1}`.

**Gap / severity:**
- LOW: media-caps JSON whitespace divergence (`{"Version":1}` vs `{"Version":
  1}`). Functionally inert (JSON parsers ignore whitespace) but not
  byte-faithful.
- MEDIUM: sender Ask request omits `SenderID`. The reference receiver does not
  require it (it auto-accepts), but a real Apple receiver may key logging /
  dedup off it and a more faithful sender should include it.

**RESULT:**
- Media-caps whitespace fixed to match `json.dumps` (`{"Version": 1}`).
- Sender now includes a `SenderID` (`Value::Data`) in the Ask request, plus
  the sender-side `parse_ask_request` test already tolerates it.

---

## 4b. mDNS service — GAP (flags value) — FIXED

**Reference** (`server.py.patched::_init_service`, `get_properties`):
- Service type: `_airdrop._tcp.local.`
- Instance name: `<service_id>._airdrop._tcp.local.` where `service_id`
  defaults to the host name.
- TXT record: `{b"flags": str(config.flags).encode("utf-8")}` — a single key
  `flags` whose value is the decimal-string of an integer.
- Server (host): `<host_name>.local.`
- Port: 8771.
- The `flags` integer: opendrop's `--flags` CLI default, which the
  verified-working patched opendrop leaves at its repo default (`0x06`). The
  flags field advertises the receiver's discoverability/capability bits to the
  sender; if the wrong bits are set the sender never surfaces us in its
  picker.

**luftlift** (`mdns.rs`, `main.rs`):
- Type `_airdrop._tcp.local.`, hostname `<name>.local.`, single `flags` TXT
  key, port 8771. All faithful. The PID-suffixed instance name
  (`<name>-<pid>`) is an intentional luftlift addition so a fresh run is
  distinguishable from a stale ghost in the Apple picker (documented in the
  rustdoc of `build_airdrop_service_info`).
- **`flags` is hardcoded to `1`** in `main.rs::receive`. opendrop's verified
  default is `0x06` (decimal 6): `0x06 = 0b110` sets the "supports receiving"
  and discoverability bits Apple's sender checks. Value `1 = 0b001` sets a
  different single bit and may not be enough for iOS 18 to surface us.

**Gap / severity:** MEDIUM. Wrong default flags can make the receiver invisible
in the AirDrop picker even when mDNS/TLS/HTTP are all correct.

**RESULT:** Fixed. `main.rs` `receive` now defaults `flags` to `0x06` (the
opendrop-verified value) and exposes a `--flags` CLI override so the operator
can experiment. `MdnsConfig` doc-comment updated to document `0x06`.

---

## 5. TLS — OK (faithful posture)

**Reference** (`server.py.patched::_init_server`, `get_ssl_context`):
self-signed cert via Python's `ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)`,
`wrap_socket(server_side=True)`. No client-cert requirement. Apple accepts
self-signed certs for AirDrop (encryption only, no WebPKI chain validation on
the AWDL link).

**luftlift** (`tls.rs`):
- `generate_self_signed_cert`: rcgen ephemeral keypair + `CertificateParams`
  with a `luftlift` SAN. Apple doesn't validate the SAN for AirDrop (it pins
  nothing on the receiver side), so the exact SAN doesn't affect interop.
- `build_server_config`: `ServerConfig::builder().with_no_client_auth()
  .with_single_cert(...)` — matches the reference's no-client-auth posture.
- `build_client_config`: `AcceptAnyCert` verifier accepts any self-signed
  server cert, mirroring Apple's lenient TLS on the sender side. Supports the
  full signature-scheme matrix so whatever Apple's receiver picks validates.
- rustls features `["ring", "std", "tls12"]` enable both TLS 1.2 (what iOS
  commonly negotiates) and TLS 1.3.

**Gap / severity:** NONE material. The cert is self-signed, no client auth,
both TLS versions available — matching the reference and Apple's expectations.
The cert CN/SAN differs from a real Mac (`luftlift` vs a hostname) but Apple
doesn't gate on it for AirDrop.

**RESULT:** No change needed.

---

## 6. HTTP handlers + content types — MINOR (406 Content-Type, Connection)

**Reference** (`server.py.patched`):
- Routes `POST /Discover`, `POST /Ask`, `POST /Upload`; 400 for unknown paths.
- `/Discover`, `/Ask` response: `200`, `Content-Length: <n>`, binary plist
  body. No explicit `Content-Type` header set by `_set_response` (Python's
  BaseHTTPRequestHandler adds `Server`/`Date` automatically).
- `/Upload` happy path: `200`, `Content-Length: 0`, `Connection: close`.
- `/Upload` unsupported content-type: `406`, `Content-Type: application/x-cpio`,
  `Content-Length: 0`, `Connection: close`.
- `/Upload` non-chunked body: `400`, `Transfer-Encoding: Chunked` (sic —
  capitalised), `Content-Length: 0`, `Connection: close`.
- `/Upload` `Expect: 100-continue`: interim `100` with `Content-Length: 0`
  before reading the chunked body.

**luftlift** (`handlers.rs`, `http.rs::format_response`):
- Routes match; unknown path → 404 (the reference returns 400; both are
  reasonable rejections of an unknown route — 404 is more semantically correct
  for "no such resource" and Apple never POSTs to unknown paths in practice).
- Discover/Ask response: `200`, auto `Content-Length`, **adds**
  `Content-Type: application/x-apple-plist` (the reference omits this; an
  extra correct Content-Type is harmless and more standards-compliant, so
  kept).
- Upload happy path: `200`, `Connection: close`. ✓
- Upload unsupported content-type: `406`, `Connection: close`, **missing**
  `Content-Type: application/x-cpio` that the reference emits.
- Upload non-chunked: NOT enforced (reference rejects with 400; luftlift
  accepts any body framing). Our more-lenient stance is functionally correct
  (Apple always sends chunked for /Upload; accepting Content-Length too just
  lets our own client and opendrop's client interop).
- Expect: 100-continue: **not handled** (see §3 — HIGH, now fixed).

**Gap / severity:**
- LOW: missing `Content-Type: application/x-cpio` on the 406 error body.
- LOW: `format_response` doesn't emit `Connection: close` by default; the
  per-handler code adds it for Upload but Discover/Ask responses don't carry
  it, so an HTTP/1.1 client could assume keep-alive is available (we close
  after every request in practice). Adding `Connection: close` everywhere
  makes the wire output match what we actually do.

**RESULT:**
- 406 response now carries `Content-Type: application/x-cpio` like the
  reference.
- `format_response` emits `Connection: close` on every response so the
  HTTP/1.1 connection-state on the wire matches reality (we close after each
  request — AirDrop opens a fresh TLS connection per POST anyway).

---

## 7. send client — OK (SenderID added, otherwise faithful)

**Reference** (`docs/airdrop-protocol.md` + the opendrop client behaviour
implied by the patched server): the sender runs `POST /Discover` → `POST /Ask`
→ `POST /Upload`. Discover/Ask bodies are binary plists with
`Content-Length` (opendrop's client uses `urllib`, which sends
Content-Length). The Ask body includes `SenderID` per the real capture.

**luftlift** (`client.rs`):
- `build_discover_request` / `build_ask_request` / `build_upload_request`
  produce the right paths, content-types (`application/x-apple-plist`,
  `application/x-dvzip`), and binary-plist bodies. `format_request` auto-adds
  `Content-Length` unless `Transfer-Encoding: chunked` is set.
- `send_files` runs the Discover → Ask → Upload sequence against any
  `AirDropTransport`, with an in-memory transport test
  (`send_files_end_to_end_succeeds_via_in_memory_transport`) that round-trips
  the client through the server handler.
- `RustlsTransport` opens a fresh TLS connection per request (matching
  AirDrop's per-POST connection model) and reads the response via
  `read_full_response` (Content-Length-delimited, which is what Apple
  receivers send).

**Gap / severity:** MEDIUM — sender Ask omits `SenderID` (see §4). Also, the
sender's `read_full_response` stopped at the first header terminator, so an
interim `1xx` (e.g. `100 Continue`) from a real Apple receiver would be
mistaken for the final response (RFC 7231 §6.2 violation).

**RESULT:**
- Sender now includes `SenderID` (`Value::Data`, 6 random bytes) in the Ask
  request body, matching the real iOS-18 capture shape.
- `read_full_response` now loops past interim `1xx` responses and returns the
  first final (non-1xx) response, per RFC 7231 §6.2.

---

## Summary of fixes landed

| Layer | Gap | Severity | Commit |
|-------|-----|----------|--------|
| §3/§6 | Expect: 100-continue not handled (deadlock risk) | HIGH | `614818e` |
| §4b | mDNS `flags` default 1 → 0x06 + `--flags` CLI | MEDIUM | `489ad0e` |
| §4/§7 | Ask request missing `SenderID` | MEDIUM | `14b78ce` |
| §4 | media-caps JSON whitespace `{"Version": 1}` | LOW | `9fa877c` |
| §6 | 406 response `Content-Type: application/x-cpio` | LOW | `9fa877c` |
| §6 | `Connection: close` on all responses | LOW | `9fa877c` |
| §3/§7 | client must skip interim `1xx` responses (RFC 7231) | MEDIUM | `71a99c4` |

Layers with no gap (§1 dvzip, §2 CPIO, §5 TLS) are left unchanged.

Baseline → final: `cargo test -p luftlift-rs` 66 → **77 passed**;
`cargo clippy -p luftlift-rs --all-targets -- -D warnings` clean throughout;
release binary builds (`cargo build -p luftlift-rs --release`).
