# PORT.md — `awdl-rs` (port of owl)  — owner: gpt-5.5

Goal: a Rust port of SEEMOO's **owl** AWDL daemon. It puts a Wi-Fi monitor
interface to work, speaks Apple Wireless Direct Link, and brings up an `awdl0`
network interface that the separate `airdrop-rs` crate uses via normal sockets.
**You do not need to touch airdrop-rs** — the only contract is the `awdl0`
netdev (IPv6 link-local, MTU 1450).

Reference C source is in `../../owl/` (read it). Our key fix is in
`owl/src/rx.c` (hardware-TSF timestamping) — port that behavior from the start,
not owl's buggy original.

## What owl does (and you must port)
1. **Interface setup**: take a Wi-Fi iface already in plain monitor mode (we run
   it externally; do NOT try to set the `NL80211_MNTR_FLAG_ACTIVE` flag — many
   drivers reject it, this was owl's `-N` mode). Open it for RX+TX (pcap or
   AF_PACKET). Create the host tun `awdl0` with the same ether addr.
2. **RX path** (`src/rx.c`): parse radiotap → 802.11 → AWDL action frames.
   Handle PSF (Periodic Sync Frame) and MIF (Master Indication Frame) and their
   TLVs (sync params, election params, channel sequence, service params, data path).
   De-encapsulate AWDL data frames and write them to `awdl0`.
3. **Election** (`src/election.c`): track the AWDL master election tree; adopt the
   highest-metric master; expose the current master + metric.
4. **Synchronization** (`src/sync.c`) — **THE critical part**:
   - Compute availability-window timing from the master's announced
     `time_to_next_aw` + `aw_counter`.
   - **Timestamp received frames with the radiotap TSFT (hardware RX time), NOT a
     userspace clock.** Bridge TSF→monotonic with a filtered offset (EMA + sanity
     clamp). This is the single most important correctness item — see
     `../../docs/awdl-sync.md` and our `owl/src/rx.c` patch. Target sync error well
     under the AWDL guard window; on a good radio (Atheros) we measured ~0.4%.
5. **Channel hopping** (`src/channel.c`, `daemon/core.c`): follow the master's
   channel sequence; default anchor **channel 44**. Use nl80211 set_channel.
6. **TX**: periodically send our own PSF/MIF; inject AWDL-framed data from `awdl0`.
   Injection can **EAGAIN** under load — size the send buffer (SO_SNDBUF) and
   queue/retry rather than drop (owl just dropped → ~32% loss on USB).

## CLI (match owl where sensible)
`awdl-rs -i <wlan> [-c 44] [-h awdl0] [-N] [-f] [-v...]`
- `-i` monitor iface, `-c` channel (6/44/149), `-h` host iface name (awdl0),
  `-N` assume already-monitor, `-f` disable RSSI filter.

## Debuggability (a primary goal — better than owl)
Use `tracing` with structured fields. Emit events:
- `peer_added`/`peer_removed` { addr, rssi }
- `election_changed` { self, master, metric, hops }
- `sync_error` { error_tu, pct, master } — every measurement, not just over-threshold
- `channel_switch` { from, to }
- `tx_inject` { ok, eagain_count }
Provide `--json` logs and ideally a small status endpoint or periodic summary
line (peers / master / sync% / channel). The Python/C stack made "why no
discovery?" agony — make it observable here.

## Milestones
1. iface setup + awdl0 up + radiotap/802.11/AWDL frame parsing (RX), structured peer logging.
2. Election + sync (hardware TSF) — log sync% so we can verify against the C owl.
3. Channel hopping + TX (PSF/MIF + data inject with backpressure handling).
4. Validate: with a real Apple device active, `airdrop-rs` (other crate) should
   discover and receive a file over the awdl0 this brings up.

Coordinate with Claude (orchestrator) for: workspace dep additions, the awdl0
contract, and anything that would touch shared files. Keep all your code under
`crates/awdl-rs/`.
