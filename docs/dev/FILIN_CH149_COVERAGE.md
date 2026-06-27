# filin task: discovery regressed — filin only covers ch 6 & 44, never 149

## Live finding (full-Rust stack, real iPhone)
After the chanseq-decode (0674ad1) + hysteresis/pin (4e91681) commits, the iPhone NO LONGER
sees luftlift in the AirDrop picker. It DID see it in the prior stack (multicast-spread only,
no Bug A/B/C). Diagnosis from filin's own debug log (`switched monitor channel channel=N`):

    167x channel=44
    166x channel=6
      0x channel=149   <-- never

Across 333 channel switches filin tuned ONLY to 6 and 44 — never 149. Real iPhone AirDrop
scans/uses the AWDL social channel 149 heavily. If filin never visits 149, it can neither
hear the iPhone's mDNS queries there nor land its (spread) announce there -> no discovery.
The mDNS announce itself is healthy and IS spread on-wire across filin's channels (confirmed
via tcpdump on awdl0), so this is purely a CHANNEL-COVERAGE problem, not an mDNS problem.

This coverage shrank exactly when hysteresis stopped the master thrash: before, filin churned
across many masters' sequences (incidentally covering more channels, likely incl. 149); now it
commits to one master whose DECODED sequence is only 6/44.

## What to determine (decisive)
Is filin's chanseq DECODE (Bug A) dropping 149, or do the adopted masters genuinely advertise
only 6/44?

1. Add a DEBUG log of the RAW channel-sequence TLV bytes (hex) for each adopted master, AND
   the full decoded 16-slot channel list. So we can see exactly what bytes came in and what
   filin produced. (e.g. `tracing::debug!(raw = %hex(bytes), decoded = ?channels, "chanseq")`.)
2. Compare filin's decode of those raw bytes against owl's `awdl_chan_num` / chanseq parse
   (owl/src/channel.c, the encoding cases) for the SAME bytes. Real Apple AWDL chanseqs are
   16 entries; the encoding field selects how each entry maps to a channel. If filin's per-
   encoding decode mishandles the 5GHz/opclass case, 149 (0x95, opclass 0x80) would be lost or
   mis-mapped to 6/44. Verify against a captured sequence.

## Fix (pick based on #2)
- If Bug A's decode is DROPPING/mis-decoding 149 (most likely): fix the encoding handling so
  the full sequence — including 149 — decodes correctly. Add a per-encoding test using a real
  Apple chanseq TLV that contains 149 and assert 149 appears in the decoded slots.
- If the masters truly only advertise 6/44: make filin cover the AWDL SOCIAL channels for
  discovery regardless — i.e. ensure the multicast announce is also transmitted on 149 (and 6,
  44) by having filin periodically visit 149, or spread the announce across the union of
  {committed master seq} ∪ {social channels 6,44,149}. (Don't break the transfer-pin: pinning
  during an active unicast upload still takes priority.)

## Constraints
- Edit ONLY crates/filin-rs. TDD, keep green (filin currently 99 tests). Clippy -D warnings
  clean, release builds. Separate commit(s), e.g.
  `fix(filin): decode full master chanseq incl. ch149` and/or
  `fix(filin): cover AWDL social channels for discovery announce`.
- Do NOT touch the running root filin pid (1798331), luftlift, the workspace Cargo.toml, or
  owl/src/election.h.

When done, state clearly: was 149 being dropped by the decode (root cause) or are the masters
6/44-only — and what you changed — so the orchestrator can rebuild+restart and have the human
retry discovery.
