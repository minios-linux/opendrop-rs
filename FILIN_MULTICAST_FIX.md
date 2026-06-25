# Task: fix filin so it transmits multicast/broadcast frames (mDNS announce)

## Symptom (confirmed live)
When the AWDL link is provided by **filin** (instead of owl), **no Apple device sees
luftlift in the AirDrop picker** — but it works fine over owl. luftlift IS writing its
mDNS announce to awdl0 (awdl0 TX counter climbs), but those frames never reach the Mac.

## Root cause
The channel-hopping work added **AW-gated unicast TX**: data frames read from the awdl0
TAP are only injected when filin is on the **same channel as the destination peer**
(`schedule::same_channel_as_peer` + the unicast guard). That's correct for UNICAST.

But luftlift's mDNS **announce is MULTICAST** (IPv6 dst `ff02::fb`, Ethernet dst
`33:33:00:00:00:fb`) — and so are IPv6 NDP solicitations (`33:33:ff:..`) and any
broadcast. A multicast/broadcast frame has **no single destination peer**, so the
"same channel as peer" lookup fails and the frame is **never injected** -> the Mac
never receives the announce -> filin's receiver is invisible.

owl does NOT gate multicast this way — it sends multicast on the current channel during
the availability window (owl `awdl_send_multicast` / the mcast path in daemon/core.c,
distinct from the unicast path).

## Fix
In the runtime TX path (crates/filin-rs/src/lib.rs `runtime::run` / the awdl0->AWDL data
injection + the AW-gated TX queue): detect multicast/broadcast destination MACs
(first octet of the Ethernet dst has the I/G bit set: `dst[0] & 0x01 == 1`, covers
`33:33:..` IPv6 multicast and `ff:ff:..` broadcast) and DO NOT apply the per-peer
same-channel gate to them. Multicast/broadcast frames should be injected on the CURRENT
channel (every availability window we're transmitting in), like owl's multicast path —
not held waiting for a per-peer channel match. Keep the unicast same-channel gating for
unicast frames only.

Reference owl's split: unicast TX (awdl_send_unicast, per-peer, gated) vs multicast TX
(awdl_send_multicast, sent on the current AW channel). Port that distinction.

## Deliverable
- Implement in crates/filin-rs/. Keep the testable decision pure (e.g. a function that,
  given dst MAC + current channel + peer table + now, decides send/hold — multicast =>
  send-on-current-channel, unicast => same-channel-as-peer gate). Add a unit test:
  a multicast dst (33:33:00:00:00:fb) is NOT peer-gated (sent on current channel even with
  no matching peer), while a unicast dst still is.
- Keep `cargo test -p filin-rs` green + `cargo clippy -p filin-rs --all-targets -- -D
  warnings` clean. Commit as `fix(filin): do not peer-channel-gate multicast/broadcast TX`.
  Rebuild release.
- After this lands, the human will swap owl->filin and verify the Mac sees luftlift over
  the full-Rust filin+luftlift stack.

Only edit crates/filin-rs/. Work autonomously.
