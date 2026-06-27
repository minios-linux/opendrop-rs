# LUFTLIFT task: pin mDNS to awdl0 + fix TXT flags (two P0 discovery blockers)

You own ONLY `crates/luftlift-rs`. TDD, commit per increment, tests + clippy green. Do NOT touch
other crates. These came from an audit vs the reference python opendrop + docs/book/07,08.

## Context
A real Mac browses luftlift via `dns-sd -B _airdrop._tcp` (PTR seen) but the AirDrop UI never
shows it and the unicast HTTPS `/Discover` fails with "No route to host". Two luftlift-side bugs:

## P0 #1 — mDNS is not pinned to awdl0; the awdl0 AAAA can be stripped
`crates/luftlift-rs/src/mdns.rs` uses the `mdns-sd` crate with **no interface pinning**. mdns-sd
auto-enumerates ALL host interfaces and, per-interface, only emits address records in the same
/64 subnet (`valid_ip_on_intf`). So on en0/wlan0 the awdl0 `fe80::/64` link-local AAAA is filtered
out → the SRV target has no resolvable AAAA → the Mac can't route to us. opendrop instead pins
zeroconf to the awdl0 address only (`server.py`: `Zeroconf(interfaces=[str(self.ip_addr)],
ip_version=V6Only, apple_p2p=...)`).

Fix: constrain the mDNS daemon to awdl0 ONLY. With mdns-sd, after creating the daemon, call
`daemon.disable_interface(IfKind::All)` then `daemon.enable_interface(IfKind::Name("awdl0"))`
(or `IfKind::Addr(<awdl0 link-local>)`) before/around registering the service. Verify (unit test
where possible; live test later) that the announce goes out on awdl0 and includes the awdl0 AAAA
for the SRV target host. If mdns-sd cannot actually multicast on awdl0 (AWDL is a P2P iface),
document that clearly and fall back to whatever interface-pinning API works — the goal is: the
records (PTR+SRV+TXT+AAAA+hostname) all go out on awdl0 with the awdl0 link-local AAAA intact, and
NOT leak on other interfaces. Compare opendrop's approach.

## P0 #2 — TXT `flags` value is wrong
`mdns.rs` sets `DEFAULT_FLAGS = 0x06` with a comment claiming it's "opendrop's verified default".
That is FALSE. opendrop uses `flags = SUPPORTS_MIXED_TYPES | SUPPORTS_DISCOVER_MAYBE = 0x08 | 0x80
= 0x88` (`opendrop/config.py` AirDropReceiverFlags). `0x06` is `SUPPORTS_DVZIP|SUPPORTS_PIPELINING`
and is missing `SUPPORTS_DISCOVER_MAYBE (0x80)` which signals the sender may run /Discover.
Fix: set `DEFAULT_FLAGS = 0x88` and correct the comment. (Confirm against
`/home/user/.local/lib/python3.10/site-packages/opendrop/config.py`.)

## Verify
- cargo test -p luftlift-rs + clippy green; build target/release/luftlift.
- Note both in PORT.md.
- Live test later (radio is wedged now): on the Mac `dns-sd -L <inst> _airdrop._tcp` then
  `dns-sd -G v6 <host>` should return the awdl0 AAAA; the AirDrop UI should surface the receiver.
