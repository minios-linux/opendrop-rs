# FILIN_AUDIT.md — owl (C) vs filin (Rust) layer-by-layer

Goal: make filin a faithful port of owl so a real Apple device registers
filin as a peer and exchanges data. Each section lists what owl does, what
filin does today, the gap, severity, and the planned fix. A RESULT note is
appended per layer as fixes land.

Reference anchors:
- owl: `owl/src/{rx,tx,election,sync,channel,peers,state,frame,wire}.c/h`, `owl/daemon/core.c`
- filin: `crates/filin-rs/src/lib.rs`

Baseline before this audit: `cargo test -p filin-rs` = 40 passed; `cargo clippy -p filin-rs --all-targets -- -D warnings` clean.

---

## 0. Radiotap RX + TX  — OK (minor)

**owl RX** (`rx.c:420-454`) walks fields with `ieee80211_radiotap_iterator`,
reads TSFT, FLAGS (FCS/BADFCS), DBM_ANTSIGNAL, then strips `it_len`
(`rx.c:512`) and strips 4-byte FCS when `FLAGS & F_FCS` (`rx.c:456-464`).

**filin RX** (`radiotap::parse_header`, lib.rs:19-85) walks present-word-0
fields with correct radiotap alignment, extracts tsft/flags/antenna_signal.
`rx::ieee80211_payload_without_fcs` (lib.rs:1281-1301) strips FCS when
`flags & 0x10` and rejects BADFCS (`flags & 0x40`). Faithful.

**owl TX** (`tx.c:297-318`) prepends a 9-byte radiotap header:
`00 00 09 00 04 00 00 00 18` (present bit 2 = Rate, value 24 = 12 Mbps).

**filin TX** (`radiotap::build_tx_header`, lib.rs:87-93) emits the identical
9 bytes; verified by test `builds_owl_compatible_tx_radiotap_header`.

**Gap:** none material. FCS strip (the original RX blocker) was fixed in
commit a75412d. RESULT: no change needed.

---

## 1. ieee80211 header / FCS / addressing  — GAP (data path)

**owl** uses `struct ieee80211_hdr` = 24 bytes (fc, dur, addr1, addr2, addr3,
seq). For DATA frames owl special-cases QoS (`rx.c:536-541`): if
`ftype==DATA && stype==QOS_DATA` it reads then strips the 2-byte QoS control
before the body, and checks the A-MSDU bit. owl's data frame case matches
`FTYPE_DATA|STYPE_DATA|STYPE_QOS_DATA` (0x0088).

**filin** (`ieee80211::parse_mac_header`, lib.rs:233-247) always takes body =
`&frame[24..]` regardless of subtype, so a QoS data frame (fc & 0x0080) leaves
the 2-byte QoS control prepended to the body — the LLC/SNAP parse then
mis-aligns and the data frame is dropped.

**Gap / severity:** MEDIUM. Apple sends QoS data (`fc=0x0888`); filin drops
all of it. Fix: detect QoS subtype, strip 2 bytes, pass through A-MSDU.

---

## 2. AWDL action/data parse + TLV types  — GAP (MIF TooShort, peer loss)

**owl** action RX (`rx.c:233-320`): parses the 16-byte `awdl_action` header,
**adds the peer to the peer table first** (`rx.c:282`), then iterates TLVs.
TLV iteration uses `read_tlv` (`wire.c:209-222`): if the final TLV is
incomplete (`READ_BYTES` runs off the end), `read_tlv` returns `OUT_OF_BOUNDS`
and the `while ((len=read_tlv(..))>0)` loop (`rx.c:295`) simply **stops** — the
peer is already registered. owl only warns if leftover bytes remain.

**filin** (`awdl::parse_tlvs`, lib.rs:448-466) returns `TooShort` if ANY
trailing TLV is incomplete, and `rx::parse_action_frame` (lib.rs:1264-1265)
treats that as a hard error for the whole frame. There is also no peer table,
so nothing is registered before TLV parsing. Real Apple MIF frames carry extra
TLVs (HT caps, ARPA, …) and trailing padding; an incomplete final TLV drops the
entire MIF → "MIF frames still fail AwdlTlv(TooShort)".

**Gap / severity:** HIGH. Fix:
1. `parse_tlvs` must stop cleanly on a short trailing TLV (return the TLVs
   parsed so far), matching owl's tolerant loop.
2. Action-header parse and TLV parse must be separable so a TLV short-fall
   never prevents peer registration.

---

## 3. ELECTION  — GAP (fabricated state — likely the main peering blocker)

**owl** (`election.c`, `election.h`): `awdl_election_state` = { master_addr,
sync_addr, self_addr, height, master_metric, self_metric, master_counter,
self_counter }. Init: `self_metric = AWDL_ELECTION_METRIC_INIT (60)`,
`self_counter = AWDL_ELECTION_COUNTER_INIT (0)`, master/sync = self, height=0.
`awdl_election_run` (`election.c:67-120`) resets self, iterates **valid**
peers, and adopts the best master by (master_counter, then master_metric),
with tie-breaks on height (shorter tree wins) and self_addr (larger wins), and
rejects peers that would form a cycle (`peer.sync_addr == self`) or exceed
`TREE_MAX_HEIGHT (10)`.

**filin** (`election` module, lib.rs:768-857): the pure `choose_sync_master`
exists and is *close* to owl's algorithm, BUT:
- Missing `self_counter` field (owl has it; the v2 TLV needs it).
- Uses `METRIC_INIT` implicitly via test values, never defines the constant 60.
- Not wired into the runtime at all. The announce builder instead fabricates
  values: `self_metric(self_addr) = 0x8000_0000 | be_bytes(addr[2..])` and
  hardcodes `master_counter = 1`, `self_counter = 1`
  (`build_election_parameters_v2_tlv_value`, lib.rs:1903-1915, and
  `build_election_parameters_tlv_value`, lib.rs:1890-1901).

**Gap / severity:** CRITICAL. filin advertises itself as a master with an
absurdly large metric (≥ 2³¹) and a fixed counter. Real Apple masters carry
small metrics and a counter that grows; a node claiming metric 0x80…… and
counter 1 is semantically nonsense. Apple will not adopt filin and likely
ignores its frames. Fix: implement real `ElectionState` with init/run, track
per-peer election state, and serialize the *actual* state in both election
TLVs.

---

## 4. PEER TABLE  — GAP (does not exist)

**owl** (`peers.c`, `peers.h`): `struct awdl_peer` per MAC = { last_update,
election state, channel sequence[16], sync_offset, name, country_code,
infra_addr, version, devclass, supports_v2, sent_mif, is_valid }. A peer
becomes `is_valid` once `sent_mif && devclass && version`
(`peers.c:62-64`). `awdl_peer_add` creates/updates + fires the "new peer"
callback. Election iterates valid peers; data RX requires the source to be a
known peer (`rx.c:361`); data TX requires the dst to be a known peer
(`core.c:218`). Stale peers are removed after `PEERS_DEFAULT_TIMEOUT (2e6 ms)`.

**filin**: no peer table whatsoever. RX action frames update an anonymous
`SyncState` and log TLVs; nothing remembers peers, their election state, their
channel sequence, version, or validity. Consequently election has no inputs and
the data path cannot gate on peer membership.

**Gap / severity:** CRITICAL. Fix: add a `peers::PeerTable` keyed by MAC with
owl-faithful `Peer` records, add/update/remove/cleanup, and `is_valid` logic.

---

## 5. SYNC + availability windows / EAW  — MOSTLY OK, one gap

**owl** (`sync.c`): aw_counter, last_update, aw_period=16, presence_mode=4;
`next_aw_tu`, `current_aw`, `current_eaw`, `sync_error_tu`, `update_last`
(mask aw_counter & 0xfffc). owl only updates sync from the **elected sync
master** (`rx.c:39` `awdl_election_is_sync_master`).

**filin** (`sync` module, lib.rs:859-918): the math is a faithful port
(update_from_master, sync_error_tu, next_aw_tu, current_aw, current_eaw all
match owl). Verified by tests.

**Gap / severity:** MEDIUM. The schedule math is correct, but filin accepts
sync updates from *any* peer (not only the master), and there is no master to
gate on because election/peer-table are absent. Fix: gate `update_from_master`
on the elected master once the peer table + election exist.

---

## 6. CHANNEL sequence  — GAP (wrong encoding/opclass in TX)

**owl** (`channel.h`): `AWDL_CHANSEQ_LENGTH=16`; `AWDL_CHAN_ENC_OPCLASS=3` (2
bytes: chan_num, opclass). `state.c:56` sets `channel.enc = OPCLASS`.
`CHAN_OPCLASS_44 = {44, 0x80}`. Default chanseq = all-16 slots = master channel
(`state.c:60`). `awdl_init_chanseq` (`tx.c:80-96`) writes count=15, encoding,
dup=0, step=3, fill=0xffff, then 16×(chan_num,opclass).

**filin** runtime (`build_channel_sequence_value`, lib.rs:1880-1888): writes
`[15, 2, 0, 3]` — **encoding 2 (LEGACY)** instead of 3 (OPCLASS) — and emits
each channel as `anchor_channel.to_le_bytes()` = `{44, 0x00}`, i.e. **opclass
byte 0x00** instead of owl's 0x80. The RX chanseq parser
(`awdl::parse_channel_sequence`, lib.rs:470-499) accepts encodings 1/2/3 but
treats them as flat channel numbers, losing the opclass byte.

**Gap / severity:** MEDIUM-HIGH. Apple validates the channel sequence TLV; an
opclass of 0 and the wrong encoding may cause Apple to reject filin's
availability advertisement. Fix: TX chanseq with encoding=3 and opclass 0x80
matching owl; carry opclass through the RX parser.

---

## 7. SCHEDULE / AW TX timing + channel switching  — GAP (blind TX, no hopping)

**owl** schedules TX on availability windows:
- `awdl_send_psf` (`core.c:178-182`): every `psf_interval` TU (110 master /
  440 slave).
- `awdl_send_mif` (`core.c:184-200`): only if `channel.current > 0`, rearmed
  to the **middle of the next EAW** (`next_aw + eaw_len/2`).
- `awdl_switch_channel` (`core.c:279-308`): at every AW boundary, switch to
  `chanseq[current_eaw % 16]` (skip null slots). This is how owl is on the
  right channel when peers are listening.
- Data TX gated by `awdl_can_send_in` / `awdl_can_send_unicast_in`
  (`schedule.c:48-80`): only inside the guard-free core of an AW and only when
  on the same channel as the peer.

**filin** runtime (`AnnounceScheduler`, lib.rs:1723-1760): fires PSF and MIF
every 110 TU blindly, with no AW/EAW alignment, no channel switching after
startup (anchor channel set once in `open_links`), and no guard logic. Data TX
is immediate (no AW gating).

**Gap / severity:** HIGH. AWDL peers only listen during their availability
windows on specific channels; filin transmitting at arbitrary times on a fixed
channel will mostly miss the peer's windows. Fix: implement channel switching
per EAW slot, gate MIF to channel>0 + mid-EAW, and gate data TX to the AW.

---

## 8. STATE machine  — GAP (no aggregated state)

**owl** (`state.h`, `state.c`): `struct awdl_state` aggregates self_address,
name, version (3.4), dev_class (MACOS=1), sequence_number, psf_interval, dst,
rssi filter, election, sync, channel, peers, stats. `awdl_init_state` wires it
all together.

**filin**: state is scattered across `run()` locals (`bridge`, `sync`,
`tx_seq`, `announce`) with no election/peer/channel aggregation.

**Gap / severity:** MEDIUM. Fix: introduce an `AwdlState` struct that owns
election + sync + channel + peers, created at startup from self_addr + anchor
channel + hostname.

---

## 9. TX frame content (PSF vs MIF TLV set)  — PARTIAL

**owl** `awdl_init_full_action_frame` (`tx.c:363-385`) TLV order: sync params
(4), election params v1 (5), chanseq (18), election params v2 (24), service
params (6), [MIF only] HT caps (7), [MIF only] ARPA (16), data path state
(12), version (21). Action header phy_tx/target_tx = `clock_time_us()`
(`tx.c:55-63`). addr3 = fixed AWDL BSSID `00:25:00:ff:94:73` (`tx.c:329`).

**filin** (`build_announce_tlvs`, lib.rs:1782-1822): TLV set/order now matches
owl (including MIF-only 7/16 and always-on 12/21). BSSID is correct
(lib.rs:1721). phy_tx/target_tx = `now_us` (lib.rs:1771).

**Gap / severity:** MEDIUM. The TLV *set* is correct, but the *values* are
wrong because they come from fabricated election state (layer 3) and wrong
chanseq encoding (layer 6). Once layers 3/4/6 are fixed, this layer mostly
inherits the correct values. Remaining: sync-params TLV fields
(`next_aw_channel`, counters) must come from real sync state (already mostly
done).

---

## Implementation order (peering-blockers first)

1. Election — real `ElectionState` + init/run (CRITICAL)
2. Peer table — `PeerTable`/`Peer` with is_valid (CRITICAL)
3. TX content — real election values + chanseq opclass encoding (CRITICAL)
4. TLV tolerance — tolerant `parse_tlvs` + separable RX (HIGH)
5. Channel switching + AW-gated TX (HIGH)
6. State aggregation + sync gated to master (MEDIUM)
7. Data path — QoS strip + A-MSDU + peer gating (MEDIUM)

(RESULT notes appended below per layer as each lands.)

---

# RESULT — implementation log

All layers below implemented in `crates/filin-rs/src/lib.rs`, committed
individually. Final state: `cargo test -p filin-rs` = **61 passed**,
`cargo clippy -p filin-rs --all-targets -- -D warnings` clean,
`cargo build --release -p filin-rs` succeeds.

## Layer 0 — radiotap RX/TX  (no change)
Already faithful after the prior FCS-strip fix (commit a75412d). TX header
(`radiotap::build_tx_header`) matches owl byte-for-byte
(`00 00 09 00 04 00 00 00 18`).

## Layer 3 — ELECTION  (commit `fix(filin): real election state matching owl`)
Rewrote `election` module as a faithful port of `owl/src/election.c`:
`ElectionState` gained `self_counter`; constants `METRIC_INIT=60`,
`COUNTER_INIT=0`, `TREE_MAX_HEIGHT=10`. `ElectionState::new`/`run`/`is_sync_master`
mirror owl's init/run/is_sync_master including cycle prevention, max-height
rejection, validity filter, and the (counter, metric, height, address)
tie-breaks. Removed the fabricated `0x80000000`-based `self_metric`.

## Layer 4 — PEER TABLE  (commit `fix(filin): add owl-faithful peer table`)
New `peers` module: `Peer` (addr, last_update, election state, 16-slot
`(chan_num, opclass)` sequence, name, version/devclass, sent_mif,
supports_v2, is_valid) and `PeerTable` with `touch` (owl `awdl_peer_add` +
`is_complete = sent_mif && devclass && version` validity recompute), `get`,
`election_views`, `remove_stale` (owl `awdl_peers_remove`, 2 s timeout).

## Layer 2 — TLV tolerance  (commit `fix(filin): tolerate incomplete trailing TLVs`)
`awdl::parse_tlvs` now stops cleanly on an incomplete trailing TLV instead
of returning `TooShort`, matching owl's `read_tlv` loop (`owl/src/rx.c:295`).
This unblocks real Apple MIF frames that carry extra TLVs + trailing padding.

## Layer 8 — STATE aggregation + sync gating  (commit `fix(filin): aggregate state…`)
New `state` module: `AwdlState` owns election + sync + peers + own channel
sequence + version/devclass + tx_seq. `apply_action` mirrors owl
`awdl_rx_action`: registers the peer, folds TLVs into peer state (election
v1/v2, chanseq, version, arpa), marks MIF, recomputes validity, re-runs
election, and **only adopts sync from the elected master** (owl `rx.c:39`).
`clean_peers` drops stale peers + re-runs election. Added
`awdl::parse_election_parameters` (v1) for owl `rx.c:111-131`.

## Layer 9/3/6 — TX content (real election + opclass chanseq)  (commit `fix(filin): wire runtime to real…`)
Announce builders now take `&AwdlState` and serialize the **real** election
state (master_addr, sync_addr, master_counter, master_metric, self_metric,
self_counter, height) in both election TLVs and the sync-params
`master_addr`. Channel-sequence TLV switched to owl **OPCLASS encoding (3)**
with opclass byte `0x80` (was encoding 2 / opclass 0). Removed fabricated
`self_metric`. ARPA TLV carries the real hostname. Runtime `run()` rewired
to `AwdlState`; periodic clean timer (1 s) added.

## Layer 7 — AW-gated MIF timing  (commit `fix(filin): AW-gated MIF timing…`)
New `schedule` module porting `owl/src/schedule.c` (`is_multicast_eaw`,
`slot_channel`, `can_send_in`). Added `SyncState::next_aw_us`. MIF is now
rescheduled to the **middle of each EAW** (`next_aw + eaw_len/2`, owl
`awdl_send_mif`) and gated to a non-null channel slot, replacing the blind
110 TU interval. PSF stays on the 110 TU master interval.

## Layer 1 — ieee80211 QoS + Layer 1 data peer gating  (commit `fix(filin): QoS data strip + peer-gated data RX`)
`ieee80211::parse_mac_header` now strips the 2-byte QoS control field for
QoS data subtypes (`fc & 0x0088`) so Apple's QoS data frames (fc `0x0888`)
de-encapsulate correctly. Data RX is peer-gated: frames from unknown sources
are dropped, matching owl `awdl_rx_data` (`owl/src/rx.c:361`).

## Known follow-ups (lower severity, not peering-blockers)
- A-MSDU aggregation (`awdl_rx_data_amsdu`) — owl supports it; filin handles
  single-MSDU only. Apple usually uses non-A-MSDU for AWDL data.
- Channel hopping per the peer's channel sequence: owl defaults to a static
  all-anchor sequence (as filin now does), so this only matters if a peer
  demands a non-static sequence. `schedule::slot_channel` + `can_send_in`
  provide the hooks.
- Unicast data TX guard gating (`awdl_can_send_unicast_in`) — multicast/data
  TX timing uses the multicast guard; unicast per-peer same-channel gating is
  scaffolded but not yet wired into the data TX path.
