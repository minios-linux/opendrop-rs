# FILIN task: re-broadcast + dwell on the adopted master's ANCHOR channel

You own ONLY `crates/filin-rs`. TDD per WORKFLOW.md, commit per increment, keep
`cargo test -p filin-rs` + clippy green. Do NOT touch other crates.

## Problem (diagnosed live, 2026-06-25)
We finally reached a real iPhone test. The AWDL cluster here anchors on its **slot-0 /
primary channel = 52** (a DFS channel; a kernel shim now lets us TX on it, so that's no longer
the blocker). filin's adopted master sequence decodes to `[52,0,44,0,0,0,0,0,6,0,44,0,...]`.

But filin only re-broadcasts the cached mDNS announce on the **hardcoded social channels
6/44/149** — confirmed via the new `/status` `rebroadcast_counts` = {149,44,6}, **never 52**.
And the injected hop sequence puts only ~6% dwell on the anchor (52). So on a 52-anchored
cluster, the iPhone (which spends its primary Availability Window on 52) almost never hears our
announce, and there isn't enough sustained presence on 52 for the HTTPS /Discover handshake.

## Fix
Make filin **adaptive to the adopted master's anchor channel** (the slot-0 / dominant channel
of the decoded master sequence), not hardcoded to 6/44/149:
1. **Re-broadcast the cached announce on the adopted master's anchor channel** in addition to
   the existing social channels. If the anchor is one of 6/44/149 already, no change; if it's
   52 (or any other), add it to the re-broadcast set dynamically.
2. **Increase dwell on the anchor channel** so there's enough continuous presence for the
   discovery + HTTPS exchange (the anchor is where the peer's primary AW is). Tune the injected
   sequence so the master's slot-0 channel gets materially more than ~6%.
3. Keep it general: read the anchor from the currently-adopted master sequence at runtime; do
   not hardcode 52.

## Verify
- `cargo test -p filin-rs` green (add tests for: anchor extracted from a decoded seq;
  re-broadcast set includes the anchor; dwell weighting toward anchor).
- After rebuild, `curl -s localhost:9930/status | jq .rebroadcast_counts` should show the
  anchor channel (e.g. "52") incrementing when the master anchors there.
- Commit in TDD increments; note the change in PORT.md.

The release binary is run from `target/release/filin`; after you build, the orchestrator will
restart the live process with your new binary.
