# filin TX Diagnosis

## Root-cause ranking

1. **Missing TX radiotap header on injected monitor frames.**
   owl prepends a radiotap header before every injected AWDL action/data frame. `owl/src/tx.c:297-318` builds a TX radiotap header with present bit 2 (`IEEE80211_RADIOTAP_RATE`) and rate value `24` (12 Mbps). `owl/src/tx.c:363-368` puts that radiotap header before the 802.11 action header, and `owl/src/tx.c:392` does the same for data frames. `owl/daemon/io.c:361-365` then injects that complete radiotap-prefixed buffer with `pcap_inject()`.

   filin currently builds bare 802.11 frames and sends them directly. `crates/filin-rs/src/lib.rs:1578-1588` calls `build_announce_frame()` and sends the returned buffer; `build_announce_frame()` at `crates/filin-rs/src/lib.rs:1741-1757` starts with the 802.11 management/action header, not radiotap. The TAP data path has the same issue at `crates/filin-rs/src/lib.rs:1672-1680`, where `ieee80211::build_data_frame()` is sent directly. `os::packet::PacketSocket::send()` at `crates/filin-rs/src/lib.rs:2245-2255` is just a raw `send()` wrapper and does not synthesize radiotap metadata.

   This is the highest-impact divergence: carl9170 monitor injection expects a radiotap-prefixed packet. If filin sends bare 802.11, the driver can accept the userspace write but fail to put the intended frame on air, which matches the live symptom: filin syncs from Apple traffic but Apple sees no filin peer and sends zero frames to filin's MAC.

2. **Incomplete owl PSF/MIF TLV set.**
   owl action frames include TLVs in this order: sync params, election params v1, channel sequence, election params v2, service params, MIF-only HT capabilities, MIF-only ARPA, data path state, version (`owl/src/tx.c:370-380`). filin currently emits only sync params, election v1, channel sequence, election v2, and service params (`crates/filin-rs/src/lib.rs:1766-1788`). Missing data path state and version are likely important for peer-table registration; missing MIF HT capabilities and ARPA may affect service/datapath readiness.

3. **Availability-window timing is approximate.**
   owl schedules PSF every `psf_interval` TU (`owl/daemon/core.c:410-415`) and schedules MIF into the middle of the next EAW (`owl/daemon/core.c:184-198`). filin now sends on a simple 110 TU periodic scheduler. This should be acceptable for getting frames heard, but it is less faithful than owl and may need refinement after TX is confirmed on-air.

4. **Election content is simplified.**
   owl keeps election state and counters coherently in `owl/src/election.c` and serializes that state in both election TLVs. filin currently uses deterministic self-master values and fixed counters. That is probably good enough for initial peer advertisement, but it is less faithful than owl and may matter once Apple starts comparing election state over time.

## Precise fix

First fix TX framing: add a small TX radiotap builder equivalent to `ieee80211_init_radiotap_header()` and prepend it to every packet sent through the monitor socket, both PSF/MIF announces and data frames. The expected prefix is:

`00 00 09 00 04 00 00 00 18`

That means radiotap v0, length 9, present bitmap bit 2 (Rate), rate value 24 (12 Mbps).

Then fill the missing owl action TLVs: data path state and version on PSF/MIF, plus HT capabilities and ARPA on MIF.
