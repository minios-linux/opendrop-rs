# FILIN task: audit fixes vs owl + book (do AFTER the neighbor-table task)

You own ONLY `crates/filin-rs`. TDD, commit per increment, tests + clippy green. From an audit of
filin vs owl (`owl/src/`) + book (docs/book/04,05). Ordered by impact.

## 1. HIGH — add the RSSI admission filter (owl has it, filin doesn't) → fixes master flapping
owl drops weak frames before admitting/updating a peer (`owl/src/rx.c:273-278`, defaults in
`owl/src/state.h:32-33`): a KNOWN peer's frame is dropped when `rssi < rssi_threshold + rssi_grace`
(-65 + -5 = -70); an UNKNOWN peer when `rssi < rssi_threshold` (-65). Book ch.5 "RSSI hysteresis"
describes exactly this: a -65 dBm edge-sync threshold with a grace bonus so the current master gets
slack — this is what PREVENTS master flapping.

filin parses `rssi_dbm` (lib.rs ~3524/3561) but only LOGS it and never gates. Every decodable
frame, however marginal, admits/refreshes a peer and can win election → the churn we observed.

Fix: in the rx path before `apply_action` admits/updates the peer, drop the frame if its RSSI is
below threshold: use -65 for an unknown peer, -70 (threshold+grace) for an already-known peer (and
ideally extra slack for the current committed master). Make threshold/grace tunable constants. Add
a CLI flag to disable filtering for testing if easy. Unit-test the decision (known vs unknown vs
master, above/below threshold). Mirror owl's semantics.

## 2. MEDIUM — saturating_sub to avoid TSF underflow
`update_from_master` (lib.rs ~1428-1429) computes `rx_hw_time_us - tu_to_us(eaw_period_tu -
time_to_next_aw_tu)` which can underflow early after start / after a TsftBridge re-lock. Same
unchecked subtraction in `next_aw_tu` (~1442) and `current_aw` (~1458). `next_aw_us` (~1452)
already uses saturating_sub — apply the same to these three. Add a regression test feeding a small
rx time / backward clock step.

## 3. LOW–MED — AWDL data-header magic should be 0x0403 (breaks TRANSFER, not discovery)
`encapsulate_ethernet_payload` (lib.rs ~712-713) writes the data-frame head field as `0u16`; owl
`awdl_init_data` writes `AWDL_DATA_HEAD = 0x0403` (`owl/src/tx.c:43`). A real Apple peer may reject
data frames with the wrong head magic, breaking the file-upload phase (discovery is action frames
only, so this doesn't affect the picker — but fix it so the actual transfer works). filin's own
decap ignores the head, which hid it in loopback tests. Set head = 0x0403 and update the affected
test vector.

## Do NOT do
- Do NOT change election METRIC_INIT/COUNTER_INIT to match the uncommitted `owl/src/election.h`
  edit (2000/2000000). filin's 60/0 is correct per canonical owl + the book; matching the edit
  would make filin wrongly seize mastership on boot.

Build target/release/filin when done; note changes in PORT.md.
