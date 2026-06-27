# FILIN task: sync-quality metrics + master-adoption hysteresis

You own ONLY `crates/filin-rs`. TDD per WORKFLOW.md, commit per increment, keep
`cargo test -p filin-rs` + clippy green. Do NOT touch other crates.

## Context (live iPhone debugging, 2026-06-25)
We finally have a clean diagnosis. With DFS unblocked (kernel shim) and anchor re-broadcast
working, a real iPhone STILL doesn't discover luftlift — even point-blank with strong signal.
The RF environment has **5 Apple devices churning as AWDL master** (anchor flipping 6<->52),
and Apple devices discover each other fine *in this same environment*. So the cluster works;
our problem is that **filin's sync isn't robust through the churn** — `mdns_rx_other` bursts
then flatlines as the master changes, so we never hold a stable-enough window for the
discovery + HTTPS /Discover handshake to complete. We need to (a) MEASURE sync quality, then
(b) make adoption stable.

## Part A — expose sync-quality metrics in /status (do FIRST)
Add to the introspection `/status` JSON (and the snapshot that feeds it):
- `tsf_offset_us` — last measured offset between our TSF and the adopted master's (the thing
  that must stay small; README target ~0.4% of the AW). Expose recent magnitude.
- `aw_alignment_pct` — fraction of availability windows where we were actually on the master's
  channel at the right time (or a proxy you can compute from the sync loop).
- `master_changes` — count of times the adopted master MAC changed.
- `master_age_ms` — time since the current master was adopted (how long we've held it).
- `last_master_macs` — small ring of recent master MACs (to see churn).
Pure counters/atomics + snapshot, same pattern as the existing introspection. Add tests.
This lets us watch the drift live (`curl :9930/status`) while the phone is on the dongle.

## Part B — master-adoption hysteresis (after A lands)
filin currently re-adopts very eagerly (we logged 5 masters cycling hundreds of times). Make it
"sticky":
- Require a candidate master to beat the current one by a **metric margin** AND persist for a
  **dwell time / N consecutive observations** before switching.
- Don't drop the current master on a single missed frame; use a timeout/grace period.
- Goal: hold one master long enough (seconds) to complete a discovery handshake, the way Apple
  devices ride through churn. Make thresholds constants you can tune.
- Verify via `/status` `master_changes` dropping sharply and `master_age_ms` growing.

## Verify
- `cargo test -p filin-rs` green (unit tests for the hysteresis decision fn + metric snapshot).
- clippy clean. Build `target/release/filin` when done; orchestrator restarts the live process.
- Note new /status fields + tuning constants in PORT.md.
