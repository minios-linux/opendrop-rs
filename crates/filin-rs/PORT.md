# PORT.md — `filin-rs` (port of owl)  — owner: gpt-5.5

Goal: a Rust port of SEEMOO's **owl** AWDL daemon. It puts a Wi-Fi monitor
interface to work, speaks Apple Wireless Direct Link, and brings up an `awdl0`
network interface that the separate `luftlift-rs` crate uses via normal sockets.
**You do not need to touch luftlift-rs** — the only contract is the `awdl0`
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
`filin-rs -i <wlan> [-c 44] [-h awdl0] [-N] [-f] [-v...]`
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
4. Validate: with a real Apple device active, `luftlift-rs` (other crate) should
   discover and receive a file over the awdl0 this brings up.

## Localhost HTTP introspection API (`--http-addr`)
A loopback-only HTTP/JSON server for live debugging — `curl | jq` instead of
grepping `/tmp/fF.log`. See `FILIN_HTTP_INTROSPECT.md` for the full rationale.

- **CLI**: `--http-addr 127.0.0.1:9930` (default), or `--no-http` to disable.
  Non-loopback addresses are rejected at config time (`ConfigError::NonLoopbackHttpAddr`)
  and the server additionally checks each connection's peer address.
- **Threading**: the server runs on its own OS thread; the AF_PACKET hot loop
  only bumps `AtomicU64` counters and (throttled ~10 Hz) refreshes an `RwLock`
  snapshot — it never takes a lock on the hot path.
- **Endpoints** (JSON):
  - `GET /status` → `current_channel`, `master_mac`, `master_seq`,
    `synced`, `peer_count`, `rebroadcast_counts: {"6":N,"44":N,"149":N}`,
    `mdns_rx_self`, `mdns_rx_other`. The `mdns_rx_other` counter is the direct
    "are we hearing the iPhone at all?" signal.
  - `GET /peers` → array of `{ mac, last_seen_ms_ago, decoded_chanseq, current_channel }`.
  - `GET /trace?n=300` → last N structured trace events from a 2000-cap ring.
  - `POST /trace` with `{"level":"off|debug|trace"}` → flips the `AtomicU8` that
    gates whether the custom `tracing` Layer pushes events into the ring (off
    by default, so the hot path is one relaxed load + compare).

Coordinate with Claude (orchestrator) for: workspace dep additions, the awdl0
contract, and anything that would touch shared files. Keep all your code under
`crates/filin-rs/`.
