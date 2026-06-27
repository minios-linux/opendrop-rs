# filin task: spread the mDNS multicast announce across the full channel-hop cycle

## Context / why
The full-Rust stack works (iPhone discovered luftlift over filin, did Discover/Ask/Upload).
BUT after a few minutes luftlift drops out of the AirDrop picker. Diagnosis (confirmed on
the wire with `tcpdump -i awdl0 udp port 5353`): luftlift's mDNS re-announce IS firing fine
(PTR/SRV/TXT flags=6/AAAA every ~60s). The problem is BELOW mDNS, in filin:

filin currently injects a multicast/broadcast data frame (the mDNS announce, dst 33:33:..
or ff:..) ONCE, on whatever channel filin happens to be on at enqueue time (your prior fix
5e221db correctly un-gated multicast so it sends at all — this builds on it). But filin
channel-HOPS following the master's channel sequence, and the iPhone listens across its OWN
channel sequence (e.g. 6/44/149). So a single-channel multicast burst only reaches the
iPhone if filin and the iPhone happen to be co-channel at that instant. Most 60s bursts
miss the iPhone's listen window -> its 120s A/SRV TTL lapses -> luftlift vanishes from the
picker.

## What to implement
Make filin transmit each multicast/broadcast TAP-originated frame on EVERY distinct channel
of the current master channel sequence over the next full hop cycle — not just once on the
current channel. So one mDNS announce from luftlift goes out on ch6 AND ch44 AND ch149 (or
whatever the adopted master chanseq contains), as filin hops through them.

Design (port owl's multicast scheduling — owl awdl_send_multicast core.c:246-277, the
multicast EAW slot logic + channel sequence in schedule.c/channel.c):
- When `data_send_decision` (or the multicast path you added) classifies a frame as
  multicast/broadcast, instead of injecting once, ENQUEUE it for transmission across the
  hop cycle: keep a small pending-multicast queue of (frame_bytes, set_of_channels_still_todo).
- At each AW/channel boundary where filin switches channel, drain any pending multicast
  frames whose todo-set contains the now-current channel: inject the frame on that channel
  and remove the channel from its todo-set. Drop the entry once its todo-set is empty (frame
  has been sent on all channels of the sequence) or after one full cycle (a TTL/age guard so
  stale frames don't accumulate).
- Channels to cover = the DISTINCT channels in the adopted master channel sequence (dedupe;
  if no master adopted, fall back to filin's anchor channel = today's single-channel behavior).
- Keep the pure decision/scheduling logic in a testable function (like your
  schedule::data_send_decision split). Avoid unbounded growth: cap the queue + age out
  entries older than one hop cycle.

## Tests (TDD, keep green)
- A multicast frame enqueued while the master chanseq = [6,44,149] is scheduled to be sent
  on all three distinct channels exactly once each across a cycle, then dropped.
- Duplicate channels in the sequence are deduped (sent once per distinct channel).
- No master adopted -> falls back to single-channel (anchor) send, unchanged behavior.
- Unicast frames are UNAFFECTED (still per-peer same-channel gated as before).
- Queue is bounded / stale entries age out (no leak if a channel never recurs).

## Constraints
- Edit ONLY crates/filin-rs. Do NOT touch luftlift or the workspace Cargo.toml.
- This is its OWN single commit: `fix(filin): spread multicast announce across the channel
  hop cycle so peers on any listen channel hear it`. (The human explicitly wants this as a
  separate commit from the luftlift re-announce-interval change.)
- `cargo test -p filin-rs` green, `cargo clippy -p filin-rs --all-targets -- -D warnings`
  clean, `cargo build -p filin-rs --release` ok.
- Do NOT touch the running root-owned filin process (pid 1763226) or owl/src/election.h.

When done, summarize the approach + test count so the orchestrator can rebuild+restart the
stack and retest whether luftlift now stays in the iPhone picker.
