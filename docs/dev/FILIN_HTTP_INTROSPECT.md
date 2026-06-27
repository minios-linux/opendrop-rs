# FILIN task: add a localhost HTTP/JSON introspection API

You own `crates/filin-rs` only. Follow WORKFLOW.md (TDD: red/green/refactor, commit per
change). Do NOT touch other crates. Keep `cargo test -p filin-rs` and `cargo clippy` green.

## Why
We debug live AWDL discovery by grepping `/tmp/fF.log`, which is painful (ANSI color codes
break greps, no live state). Add a small **loopback-only** HTTP server that exposes filin's
current state + a runtime-toggleable ring-buffer trace, so the orchestrator can `curl | jq`
instead of grepping.

## Requirements
- Bind **127.0.0.1 only** (loopback). Default port **9930**. Make it configurable via a CLI
  flag (e.g. `--http-addr 127.0.0.1:9930`) and allow disabling it. Off-box surface = none.
- Use a lightweight dependency already in the workspace if possible; if you must add one,
  prefer `tiny_http` or a hand-rolled `std::net::TcpListener` + minimal HTTP — do NOT pull in
  a full async runtime if filin isn't already async. Justify the choice in the commit message.
- The HTTP handler must NOT block or lock the AF_PACKET inject/receive hot loop. Read from a
  snapshot (e.g. `RwLock`/`ArcSwap` updated on state change) and atomic counters bumped on the
  hot path. Add tests for the snapshot/counter logic.

## Endpoints (JSON)
- `GET /status` → at minimum:
  `{ current_channel, master_mac, master_seq (the final injected sequence), synced (bool),
     peer_count, rebroadcast_counts: {"6":N,"44":N,"149":N, ...},
     mdns_rx_self, mdns_rx_other }`
  where `mdns_rx_self`/`mdns_rx_other` count mDNS (5353) frames seen on awdl0 from our own
  address vs others (this directly answers "are we hearing the iPhone at all").
- `GET /peers` → array of `{ mac, last_seen_ms_ago, decoded_chanseq, current_channel }`.
- `GET /trace?n=300` → last N structured trace events from a fixed-size ring buffer (cap ~2000;
  oldest dropped). Each event = `{ ts, level, target, msg, fields }`.
- `POST /trace` with body `{"level":"off|debug|trace"}` → flips an `AtomicU8` that gates whether
  events are pushed into the ring (off by default to keep it cheap).

## Implementation hint
A custom `tracing` Layer that, when the atomic level is enabled, serializes the event into the
ring `Mutex<VecDeque<Event>>`. Counters are plain `AtomicU64`. The status snapshot is a struct
updated wherever you already update channel/master/peer state.

## Done = 
- new tests green, clippy clean, committed in TDD increments
- `curl -s localhost:9930/status | jq` returns live data while filin runs
- a short note in `crates/filin-rs/PORT.md` documenting the endpoints + the CLI flag
