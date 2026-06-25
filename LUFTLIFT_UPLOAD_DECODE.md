# luftlift task: fix the iPhone /Upload decode (empty "." file bug)

## Symptom (live, real iPhone over the full-Rust filin+luftlift stack)
A real iPhone ("iPhone42", file_count=1) AirDropped a file. Discover+Ask+Upload all
completed and ~2500 data packets arrived, but the receiver log shows:

```
received file file=. bytes=0
Upload complete count=1
failed to save file=. error=Is a directory (os error 21)
```

So the dvzip->CPIO decode yielded ONLY the leading "." directory entry (0 bytes), NOT
the real file. A "missing CRLF terminator" WARN was also seen during the chunked read.

IMPORTANT: the Mac's 2 JPEGs decoded FINE earlier over the same code. So this is specific
to the iPhone's chunk framing / a body truncation, not the core dvzip/CPIO logic (which
matches the Python reference `opendrop-patch/dvzip.py` byte-for-byte).

## Two bugs to fix

### Bug 1 — chunked body silently truncated (PRIME SUSPECT)
`crates/luftlift-rs/src/server.rs::read_chunked_stream` (~line 226) does:
```rust
let size = usize::from_str_radix(size_str.split(';').next().unwrap().trim(), 16).unwrap_or(0);
if size == 0 { /* treat as last-chunk, break */ }
```
The `unwrap_or(0)` means ANY chunk-size line that fails to parse as hex (a partial read, a
stray byte, a CRLF desync) is silently treated as the `0` last-chunk => the body is
truncated after the first chunk. The first chunk contains only the start of the dvzip
(the first zlib block = the "." CPIO header), so the real file is dropped. This matches the
symptom exactly and is consistent with the "missing CRLF terminator" warning.

Fix: read the FULL multi-chunk body robustly. Distinguish a genuine `"0"` last-chunk from a
parse FAILURE — on a malformed size line, do NOT silently end the body; log loudly (and/or
error) so truncation is visible, and make the reader resilient to the real Apple framing.
Apply the same care to the pure `decode_chunked` in `chunked.rs` if it shares the flaw.
Verify the reader keeps consuming chunks until the real terminating `0\r\n\r\n`.

### Bug 2 — directory / "." CPIO entries saved as files
The decode returns the "." directory entry and the server tries to write it as a file
(=> "Is a directory"). Real Apple ODC CPIO archives lead with a "." directory entry (and
may contain other directory entries) before the regular file(s).

Fix: SKIP directory entries. In `cpio.rs` the ODC header has `c_mode` at offset 18 (6 octal
chars). A directory has the S_IFDIR bit set (mode & 0o170000 == 0o040000). Filter these out
in `read_odc_cpio` or in `decode_upload_body` (handlers.rs) so only regular files are
extracted/saved. Also defensively skip any entry whose name is "." or "..".

## Also add: raw /Upload body capture (for offline diffing)
Add an env-gated dump of the raw decoded request body in the /Upload path, e.g. if
`LUFTLIFT_DUMP_UPLOAD` is set, write the raw body bytes to that path. That lets us run
`python3 opendrop-patch/dvzip.py /tmp/upload.bin` offline next live test to compare our
decode vs the reference and confirm the framing. Keep it cheap and off-by-default.

## Regression test (must reproduce the real shape)
Add a test that builds an Apple-style upload: an ODC CPIO with a LEADING "." directory
entry (mode 040755, 0 bytes) followed by a real regular file with content, dvzip-framed
across MULTIPLE zlib blocks AND delivered as MULTIPLE HTTP chunks. Assert:
- the real file IS extracted with its exact bytes,
- the "." / directory entry is NOT returned as a saveable file,
- no body truncation.

## Done criteria
- `cargo test -p luftlift-rs` green, `cargo clippy -p luftlift-rs --all-targets -- -D warnings`
  clean, `cargo build --release -p luftlift-rs` ok.
- Commit as green(luftlift): ... staged TDD (red then green) per WORKFLOW.md.
- Only touch crates/luftlift-rs. Do NOT touch the running root-owned receiver process.

When done, summarize what the root cause turned out to be (chunked truncation vs CPIO
filtering vs both) so the orchestrator can update handoff.md.
