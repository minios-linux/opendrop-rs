# Task: keep luftlift visible in the AirDrop picker (periodic mDNS re-announce)

## Symptom (confirmed live)
A `luftlift receive` that has been running more than ~2 minutes DISAPPEARS from the
Apple AirDrop picker, even though the process is still up and healthy. Restarting it
makes it reappear (and transfers work) for a couple minutes, then it vanishes again.

## Root cause
`mdns::announce()` calls `daemon.register(info)` exactly ONCE and never refreshes.
mdns-sd's record TTLs are: A/SRV = **120s**, PTR/TXT = 4500s (mdns-sd defaults; the
setters are `pub(crate)` so we can't change them). So the address/port (A/SRV) records
**expire 120s after the single announcement**. mdns-sd does respond to incoming queries,
but over a quiet/flaky AWDL link the Apple device's refresh queries (or our responses)
don't reliably get through — so once the 120s A/SRV TTL lapses with no refresh, the
device can no longer resolve the receiver and it drops from the picker.

## Fix
Add a PERIODIC re-announcement so the records are refreshed proactively (independent of
whether the browser queries). In the `receive` path, every ~60-90s (comfortably under
the 120s A/SRV TTL), re-announce the same service so fresh A/SRV/PTR/TXT go out.

Implementation notes:
- Determine the right mdns-sd mechanism: calling `daemon.register(info.clone())` again
  on the SAME ServiceDaemon should re-announce the service (mdns-sd treats a re-register
  of an existing service as a re-announcement / refresh). Verify this is the behavior;
  if mdns-sd offers a more explicit refresh API in 0.11, use that. Do NOT unregister
  first (that sends a TTL=0 goodbye and would make it flicker out).
- luftlift `receive` is std-thread based. Spawn a small background thread (or use the
  accept loop with a periodic timer) that re-announces every ~60s while the receiver
  runs. Keep the existing ServiceDaemon + ServiceInfo handles. Make sure the graceful
  SIGINT/SIGTERM goodbye still fires (stop the re-announce thread on shutdown).
- Make the interval a const (e.g. `REANNOUNCE_SECS = 60`). Optionally a CLI override,
  but a sane default is fine.

## Deliverable
- Implement in crates/luftlift-rs/. Add a test where feasible (e.g. the re-announce
  helper calls register again on the daemon without error; or a timing/decision unit
  test for the re-announce interval). Keep `cargo test -p luftlift-rs` green +
  `cargo clippy -p luftlift-rs --all-targets -- -D warnings` clean.
- Commit as `fix(luftlift): periodically re-announce mDNS so receiver stays in AirDrop picker`.
  Rebuild release.

Only edit crates/luftlift-rs/. Work autonomously.

---
(Minor, secondary — note only, do if quick: a live request once hit
`connection error: body read error: missing CRLF terminator`. If you can see an obvious
cause in the chunked/body reader, fix it; otherwise just leave a NOTE in the code/commit
— it did not block the successful transfers.)
