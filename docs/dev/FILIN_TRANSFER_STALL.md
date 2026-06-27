# filin task: fix the unicast /Upload stall (channel thrash + bogus chanseq + hop-during-transfer)

## Live evidence (full-Rust stack, real iPhone uploading a photo to luftlift)
The luftlift /Upload decode is now FIXED, so the receiver correctly reports the real
problem instead of silently saving an empty file:

    WARN luftlift_rs::server: connection error error=chunked body: EOF before last-chunk (truncated)

i.e. the unicast TLS upload stream is being CUT OFF mid-body. The human saw it "stall
mid-send", and retries got "a little further" then stuck again. Discover + Ask succeed
(iPhone sees luftlift in the picker); the sustained unicast /Upload does not complete.

filin log (`/tmp/fF.log`, RUST_LOG=filin_rs=info) shows the smoking gun — filin re-adopts a
master channel sequence ~44 times in ONE minute (≈ every 1.4s), flip-flopping between
SEVERAL DIFFERENT sequences:

    adopted master channel sequence master=[174, 232, 206, 171, 81, 217]
    adopted master channel sequence master=[202, 83, 223, 212, 135, 216]
    adopted master channel sequence master=[206, 71, 161, 75, 223, 188]
    adopted master channel sequence master=[146, 197, 245, 67, 95, 175]

Two separate bugs are visible here:

### Bug A — bogus channel numbers (chanseq TLV decode)
174, 232, 206, 245, 67, 95… are NOT valid Wi-Fi channels (valid: 2.4GHz 1-13; 5GHz
36/40/44/48/.../149/153/157/161/165). Your own multicast-spread tests used 6/44/149, but the
LIVE decode yields garbage. Strong suspicion: filin ignores the AWDL channel-sequence TLV
ENCODING field and reads raw bytes. AWDL chanseq encodes entries per an encoding type
(legacy / opclass+channel pairs / channel-only). owl decodes this in src/channel.c +
src/sync.c (awdl_chan / chanseq parsing, the `enc` field). Port that: honor the encoding
field and extract REAL channel numbers. If filin hops the radio to invalid channels it can
NEVER be co-channel with the iPhone for a sustained transfer — this alone could be the stall.
Verify against a captured frame and against owl's decode of the same bytes.

### Bug B — master-adoption thrash (no stability/hysteresis)
Multiple Apple devices are on air, and filin re-adopts whichever master it last heard ~44×/min,
constantly changing its channel schedule. Even with correct channels, this churn means filin
is never predictably co-channel with the active peer. Port owl's master-stability: keep the
current master unless a challenger persistently wins the election by a margin over several
frames / a debounce window (owl election.c compare + the sync top-state). Stop re-adopting on
every frame. (Cycle-19 lesson: on a contended link, master STABILITY beats chasing.)

### Bug C — hopping away during an active unicast transfer (the human's hypothesis)
THE key fix for receive: while a unicast data flow with a peer is ACTIVE (an AirDrop /Upload
in progress — i.e. we are RX'ing unicast data frames from that peer, or there's an open TCP
conn on the awdl0 side), filin should PIN to that peer's current channel and SUPPRESS both
master re-adoption (Bug B) and channel hopping until the transfer completes or the peer ages
out. A bulk unicast TCP/TLS stream cannot survive filin leaving the channel. Implement a
"sticky transfer channel": when unicast RX from a peer is seen within the last ~N AWs, hold
that channel; release after an idle timeout.

## Priority / approach
Lead with A (correct channels) and C (pin during transfer) — together they should let the
upload complete. B (hysteresis) hardens it. Keep the pure decision logic testable (like your
data_send_decision / MulticastSpread split). TDD, keep tree green.

## Tests
- chanseq decode: a real/synthetic chanseq TLV with the opclass-or-encoding field decodes to
  valid channels (e.g. 6/44/149), NOT raw garbage bytes; cover each encoding the TLV supports.
- transfer pin: given recent unicast RX from peer P on channel C, the channel scheduler holds
  C and does not switch / does not re-adopt a different master until an idle timeout elapses.
- hysteresis: a single challenger frame does NOT flip the master; sustained wins over the
  debounce window DO.
- existing multicast-spread + unicast-gating tests stay green.

## Constraints
- Edit ONLY crates/filin-rs. Separate commit(s), e.g.
  `fix(filin): decode AWDL channel-sequence encoding to real channels`,
  `fix(filin): pin to active-transfer peer channel + master-adoption hysteresis`.
- `cargo test -p filin-rs` green, clippy -D warnings clean, release builds.
- Do NOT touch the running root filin pid (1785176), luftlift, the workspace Cargo.toml, or
  owl/src/election.h.

When done, summarize the root cause(s) + which of A/B/C you changed so the orchestrator can
rebuild+restart and have the human retry the iPhone photo upload.
