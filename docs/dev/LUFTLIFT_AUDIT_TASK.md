# Task: Systematic faithfulness audit of luftlift-rs

Audit the Rust crate `crates/luftlift-rs/` — a port of the Python AirDrop tool in
`opendrop-patch/`. Goal: make luftlift byte-for-byte faithful to real AirDrop so it
interoperates with Apple devices. This is a fresh-context, unbiased audit: do NOT
assume the implementation is correct — verify each layer against the reference.

## References
- `opendrop-patch/` — patched Python opendrop: `server.py.patched`, the opendrop
  client/protocol/util modules, `dvzip.py`.
- `docs/airdrop-protocol.md`, `docs/awdl-sync.md`.

## Layers to audit (for each: reference behavior, luftlift behavior, gap, severity, fix)
1. **dvzip framing** — `[u32 BE length][zlib block]` repeated (`dvzip.py`): framing,
   compression, multi-block handling.
2. **ODC CPIO** — magic `070707`, all octal header fields (dev, ino, mode, uid, gid,
   nlink, rdev, mtime, namesize, filesize), name + NUL, padding, `TRAILER!!!` —
   encode AND decode.
3. **Chunked HTTP** — requests AND responses: `Transfer-Encoding: chunked` with no
   `Content-Length`, `Expect: 100-continue` handling, exact header casing/values.
4. **plist Discover/Ask/Upload** — EXACT field names, types, presence in both
   requests AND responses (ReceiverComputerName, ReceiverModelName,
   SenderComputerName, SenderModelName, BundleID, ConvertMediaFormats, Files array
   with FileName/FileType/FileBomPath, etc.) vs `server.py.patched`.
4b. **mDNS service** — `_airdrop._tcp` service type/domain, TXT record keys, the
   AirDrop capability `flags` integer value, port, instance name.
5. **TLS** — self-signed cert subject/SAN/validity Apple expects, cipher suites/TLS
   versions, client-auth posture.
6. **HTTP handlers + content types** — status codes, `application/x-apple-plist`,
   `application/x-dvzip` vs `x-cpio`, Content-Length behavior, `/Discover` `/Ask`
   `/Upload` routing.
7. **send client** — request construction faithful to a real Apple sender.

## Deliverable
1. Write `LUFTLIFT_AUDIT.md` at the repo root — one section per layer
   (reference behavior, luftlift behavior, gap, severity, fix).
2. THEN implement the fixes. TDD where practical. Keep `cargo test -p luftlift-rs`
   green and `cargo clippy -p luftlift-rs --all-targets -- -D warnings` clean. Commit
   each fix separately. Rebuild release. Append a RESULT note per layer.

The human is asleep and wants the most complete / faithful implementation possible —
work through ALL layers autonomously, do not stop early. Only edit
`crates/luftlift-rs/` (filin-rs is owned by another agent; do not touch it).
