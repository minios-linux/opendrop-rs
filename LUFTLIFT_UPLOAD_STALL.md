# Task: fix luftlift /Upload stall with a real Apple sender (Expect: 100-continue regression?)

## Symptom (live, real Mac sender over a working owl AWDL link)
- `POST /Discover` → handled, receiver appears in the Mac picker. ✅
- `POST /Ask` → handled, auto-accepted (logged `Ask request — auto-accepting sender="Andrew's MacBook Pro" file_count=1`). ✅
- `POST /Upload` → **never completes**. awdl0 RX climbs in one burst then FREEZES; no
  "Upload complete"/"saved file" log line ever appears; no file lands in the output dir.
  The receiver logs NOTHING about /Upload (no 100-continue, no body read, nothing).

## Key signal: this is a REGRESSION
The OLD luftlift (before the overnight audit fixes) **successfully received an /Upload**
from this same Mac earlier (RX climbed steadily, "upload complete bytes=..."). The NEW
luftlift stalls at /Upload. The luftlift->luftlift round-trip PASSES (incl. 512KB), so
the bug only triggers with a REAL Apple sender — which differs from our own client in one
big way: **a real Apple sender uses `Expect: 100-continue` on the chunked `/Upload` POST**.

## Prime suspect
Commit 614818e "handle Expect: 100-continue" rewrote the server request read into
read_headers + (send 100) + read_body. Likely the /Upload path now mishandles the
Apple-style flow: e.g. it sends `100 Continue` at the wrong time, or after sending 100 it
doesn't correctly read the **chunked** Upload body, or it blocks waiting on something so
the Mac's data never gets consumed and the connection stalls.

## What to do
1. Re-examine handle_one_connection / handle_tls_connection + handle_upload + the
   read_headers/read_body split (crates/luftlift-rs/src/server.rs, handlers.rs) against
   `opendrop-patch/server.py.patched::handle_upload` (it handles `Expect: 100-continue`
   by sending an interim `HTTP/1.1 100 Continue` THEN reading the chunked body). Make the
   /Upload path robustly: read headers → if `Expect: 100-continue` send `100 Continue`
   → then read the **chunked** body fully → decode dvzip → write files → respond 200.
2. Add a regression test that exercises the FULL Apple-style Upload: a request with
   `Transfer-Encoding: chunked` AND `Expect: 100-continue`, a real dvzip(CPIO) body split
   across multiple chunks, and assert the server emits `100 Continue` before the body and
   then decodes+writes the file. (The existing round-trip didn't send Expect:100-continue
   — that's the gap.)
3. Keep `cargo test -p luftlift-rs` green + clippy `--all-targets -- -D warnings` clean.
   Commit as `fix(luftlift): robust Expect:100-continue chunked /Upload` (or similar).
   Rebuild release.

If after review the 100-continue path looks correct, the next suspect is the chunked
body reader interacting with TLS StreamOwned buffering (does read_body consume bytes
already buffered by read_headers?). Verify no bytes are lost between the header read and
the body read on the TLS path.

Only edit crates/luftlift-rs/. Work autonomously.

---

## SECOND task (quick, do this one first): PID in the AirDrop picker name
The AirDrop picker displays the receiver's `ReceiverComputerName` (sent in the /Discover
and /Ask responses), NOT the mDNS instance name. We earlier put the PID in the mDNS
INSTANCE name (`luftlift-<pid>`) but that does not show in the picker — the human sees
just "luftlift" and can't tell fresh runs from stale ghosts.

Fix: make the receiver's `ReceiverComputerName` include the PID by default, e.g.
`luftlift-<pid>` (or honor a `--name` CLI override). Set it where ReceiverConfig.computer_name
is built for `receive` (main.rs) so the /Discover + /Ask responses carry it, and it shows
in the Apple picker. Keep the mDNS instance name as-is. Add/keep a test. Commit as
`fix(luftlift): put pid in ReceiverComputerName so picker shows fresh runs`.
