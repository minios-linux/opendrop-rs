# FILIN task: populate the system IPv6 neighbor table for AWDL peers (likely THE discovery bug)

You own ONLY `crates/filin-rs`. TDD, commit per increment, keep tests + clippy green. Do NOT
touch other crates.

## The discovery (2026-06-25, via the Mac as an observable AWDL peer + docs/book/07)
**mDNS discovery WORKS** — a real Mac's `dns-sd -B _airdrop._tcp` lists luftlift over awdl0. So
the AWDL link + multicast mDNS are fine. But the AirDrop UI never shows us, and a manual
`curl https://[our-awdl0-ipv6]:8771/` FROM the Mac fails with **"No route to host"**. The
AirDrop `/Discover` step is a **unicast** HTTPS connection, and unicast needs the peer's
**IPv6 ↔ MAC mapping in the system neighbor table.**

`docs/book/07-service-discovery-addressing.md` explains the mechanism: AWDL does NOT use NDP.
Instead, **on receiving an action frame, a node derives the sender's link-local IPv6 from the
source MAC (RFC 4291 modified EUI-64) and adds {IPv6, MAC} straight to the system neighbor
table.** owl does exactly this (see `owl/src/state.h` neighbor add/remove hooks + its daemon
netlink code; compare `owl/src/`). **filin does NOT** — `grep -rn neigh crates/filin-rs/src`
returns nothing. That missing step almost certainly breaks all unicast awdl0 connectivity →
AirDrop can never complete past mDNS.

## Implement
1. On accepting/adding a peer (when filin processes an action frame with the peer's source MAC),
   derive the peer's link-local IPv6 via the modified-EUI-64 method:
   `fe80::(mac0 ^ 0x02):mac1:mac2:ff:fe:mac3:mac4:mac5`. (filin likely already derives its OWN
   awdl0 address this way — reuse/extract that.)
2. Add a neighbor entry on the host interface (awdl0): `{ip: derived_v6, dev: awdl0, lladdr:
   peer_mac, state: STALE or REACHABLE}` via netlink `RTM_NEWNEIGH` (preferred) or, if simpler
   and acceptable, by shelling out to `ip -6 neigh replace <v6> lladdr <mac> dev awdl0`.
   Look at how owl does it and mirror the semantics.
3. Remove/expire the entry when the peer is evicted (`clean_peers`), mirroring owl's remove hook.
4. Make it robust: don't crash if the neigh add fails (log + continue); avoid churning the table
   (only add/update when the mapping changes).

## Verify
- Unit-test the EUI-64 derivation against known vectors (and round-trip a couple MACs).
- Unit-test the add/remove decision logic (added on new peer, removed on eviction).
- `cargo test -p filin-rs` + clippy green; build `target/release/filin`.
- (Live test later, when the radio is back: `ip -6 neigh show dev awdl0` should list peers;
  then the Mac's `curl https://[our-v6%awdl0]:8771/` should connect instead of "No route to
  host", and `discover_count` should finally increment.)

NOTE: the carl9170 radio is currently hard-wedged (USB -110) so live testing is paused — focus on
the code + tests; the orchestrator will live-test when the radio recovers.
