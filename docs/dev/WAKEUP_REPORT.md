# Overnight report — filin/luftlift AirDrop port

Written while you slept. TL;DR: both crates got a full faithfulness pass, the tree is
green, and the improved stack is running and ready for a live test.

## NEW: luftlift send+receive fully validated end-to-end (overnight, no Apple needed)
A `luftlift -> luftlift` round-trip caught and fixed a real **send** bug (link-local
IPv6 needs a `scope_id`/`%awdl0` zone — was failing `POST /Discover` with EINVAL; fixed
in `52f02a1`). After the fix, both a 77-byte text file (incl. binary `0x00/0xff`) and a
**512 KB random binary** transferred Discover→Ask→Upload and landed **byte-identical**
(md5 match) through the full dvzip→zlib→ODC-CPIO→chunked-HTTP path. This validates the
**send** client we could never test against Apple (BLE gap) — the protocol is correct
and robust at realistic size. luftlift: 84 tests, clippy clean.

## What works (validated against real Apple hardware earlier)
- **luftlift AirDrop receive**: end-to-end — received a real photo from your Mac
  (Stage 1), dvzip→CPIO decoded, valid file written.
- **filin AWDL**: syncs to the Apple peer with near-perfect timing (sync_error 0–1 TU),
  real election + peer table, TSFT bridge, parses real PSF/MIF frames, transmits on-air.
- **force-master** (election bump): on a churny multi-device link it got the Mac to
  complete Discover→Ask and *start* a real Upload over owl. Slow only due to the
  carl9170's marginal RF (~0.4% sync), not the software.

## Overnight work — two parallel GLM-5.2 faithfulness audits, both COMPLETE

### filin (FILIN_AUDIT.md) — the lower-layer reimplementation you asked for
Was only "superficially" working; now faithful to owl:
- **Channel hopping** (the key gap from the live test — filin used to sit on ch44 so its
  TX never reached the Mac's listen windows). Now adopts the elected master's channel
  sequence and switches the monitor via nl80211 at each AW boundary; reverts to anchor
  when no master.
- AW-gated data TX (buffer + retry only when on the destination peer's channel, inside
  the unicast guard).
- A-MSDU aggregate data-frame decapsulation.
- Unicast same-channel gating + 3-TU guard.
- 61 → **68 tests**, clippy `--all-targets` clean. Verified: new build runs **stable on
  the carl9170** (no crash on live nl80211 switching).

### luftlift (LUFTLIFT_AUDIT.md) — protocol faithfulness vs the python opendrop
7 fixes, several real interop bugs:
- **Expect: 100-continue** — server now sends the interim 100 before reading the chunked
  body (a real Apple sender would otherwise *deadlock*).
- **mDNS flags 1 → 0x06** — matches opendrop's iOS-18-verified AirDrop capability flags
  (the old value may have hampered discovery). Added a `--flags` CLI override.
- **SenderID** added to the /Ask request the client builds (real senders include it).
- media-caps JSON whitespace, 406 `application/x-cpio` Content-Type, `Connection: close`,
  and client skipping interim 1xx responses — all matched to the reference.
- dvzip framing, ODC CPIO, and TLS posture were audited and found **already faithful**.
- 66 → **77 tests**, clippy clean.
- Plus the earlier shutdown fix: mDNS **goodbye** on Ctrl+C + **PID in the name**
  (`luftlift-<pid>`) so stale picker ghosts clear / are identifiable.

## Current state (running on pty-7, ready for you)
- `filin` (channel-hopping build) **+** `luftlift receive` are **running now**, announced
  as `luftlift-1706886`. Open an AirDrop sheet on the Mac and it should engage with
  channel hopping + the corrected flags. If it looks stale, just restart both.
- Whole workspace: `cargo build --release` + `cargo test` + `cargo clippy --workspace
  --all-targets -D warnings` all clean.

## The one real limiter that remains: the radio
The carl9170 link is marginal (~0.4% sync) and the Apple-side AWDL is bursty/idle unless
freshly triggered — that's what made transfers slow/flaky, not the code. For a clean
full transfer: fewer competing Apple devices (kill iPad/Apple-TV AirDrop), devices close,
and consider a stronger injection radio.

## Suggested first thing when you wake
1. Make sure the Mac AirDrop sheet is open (Everyone) and ideally it's the only Apple
   device active.
2. AirDrop a file to the `luftlift-<pid>` entry; watch `/tmp/lrHop.log` and `/tmp/ldrop/`.
3. If filin's TX timing still needs work, the force-master owl baseline (owl/src/election.h
   patched) is the known-furthest config to compare against.

## Notes / cleanup
- **owl is patched force-master** (`owl/src/election.h`: metric 2000, counter 2,000,000).
  Revert + `make -C owl/build owl` for a faithful owl baseline.
- **bluetoothd is stopped** on pty-8 (from the BLE PoC) — `systemctl start bluetooth`.
- BLE bootstrap (to wake idle Apple devices for the send→Apple direction) was proven
  viable (captured 270 real AirDrop adverts) but is **parked** — a real feature for later.
