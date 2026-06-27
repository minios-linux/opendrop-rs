# LUFTLIFT task: add a localhost HTTP/JSON introspection API

You own `crates/luftlift-rs` only. Follow WORKFLOW.md (TDD: red/green/refactor, commit per
change). Do NOT touch other crates. Keep `cargo test -p luftlift-rs` and `cargo clippy` green.

## Why
We debug live AirDrop discovery by grepping `/tmp/lrF.log`, which is painful. Add a small
**loopback-only** HTTP/JSON introspection API exposing luftlift's receiver state + a
runtime-toggleable ring-buffer trace, so the orchestrator can `curl | jq` instead of grepping.

## Requirements
- Bind **127.0.0.1 only**. Default port **9931**. Configurable via CLI flag
  (e.g. `--http-addr 127.0.0.1:9931`) and disablable. This is SEPARATE from the AirDrop HTTPS
  listener on 8771 — do not entangle them; the AirDrop server keeps serving Apple peers.
- The handler must not block the receive path. Snapshot + atomic counters, same as the filin
  task. Add tests for the snapshot/counter logic.
- Reuse whatever HTTP machinery luftlift already has if it's clean to do so; otherwise a minimal
  `tiny_http` / `std::net` server. Justify any new dep in the commit message.

## Endpoints (JSON)
- `GET /status` → at minimum:
  `{ receiver_name, mdns_announced (bool), https_port, last_announce_ms_ago,
     mdns_query_rx (count of inbound mDNS queries we've received),
     discover_count, ask_count, upload_count, last_peer }`
  The `mdns_query_rx` + the *_count fields let us see how far an iPhone got
  (Discover -> Ask -> Upload) without tailing the log.
- `GET /trace?n=300` → last N structured trace events from a fixed-size ring buffer (cap ~2000).
- `POST /trace` with `{"level":"off|debug|trace"}` → gates pushing events into the ring
  (off by default).

## Implementation hint
Same as filin: a custom `tracing` Layer feeding a `Mutex<VecDeque<Event>>` when enabled, plain
`AtomicU64` counters bumped in the HTTP/AirDrop handlers (Discover/Ask/Upload) and the mDNS
receive path, and a snapshot struct for `/status`.

## Done =
- new tests green, clippy clean, committed in TDD increments
- `curl -s localhost:9931/status | jq` returns live data while luftlift runs
- a short note in `crates/luftlift-rs/PORT.md` documenting the endpoints + the CLI flag
