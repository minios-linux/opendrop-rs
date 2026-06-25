# filin task: stop wasting channel hops — re-broadcast the announce on every social-channel visit + denser 149

## Problem (live, human's insight — confirmed)
The iPhone still doesn't see luftlift in the picker even though filin now visits 149. Root
cause = the mDNS announce and the channel hops don't coincide, so most hops to 149 are wasted:

1. Thin 149 coverage. Live committed sequence:
   `decoded=[6,0,44,0,...] injected=[149] final=[6,149,44,0,0,0,0,0,6,0,44,0,0,0,0,0]`
   149 is in only 1 of 16 slots (~6% dwell). ensure_social_coverage only guarantees *one*
   slot per social channel.

2. Announce cadence (10s) >> hop rate. luftlift emits an mDNS announce every ~10s. The
   multicast spread broadcasts each announce once per channel then DROPS it. So between
   luftlift's 10s announces, every hop to 149 carries NO announcement. The iPhone (listening
   on 149) only sees luftlift if it happens to listen during the rare instant filin is BOTH
   on 149 AND still holding a fresh (un-dropped) announce. Result: discovery almost never
   lands.

We confirmed (tcpdump on awdl0) that filin RXes ZERO inbound mDNS queries from the iPhone in
15s — so we can't rely on query-driven responses either; the unsolicited announce must be on
149 when the iPhone is there.

## Fix — make every social-channel hop count

### Part A (priority): cache + periodically re-broadcast the announce on social channels
filin reads luftlift's multicast mDNS frames from the awdl0 TAP and injects them on the
monitor. Today each is spread once then dropped. Instead:
- CACHE the most recent multicast/broadcast mDNS frame per multicast group (refresh the cache
  whenever a new one arrives from the TAP — keep just the latest, bounded).
- RE-INJECT the cached announce whenever filin visits a social channel (6/44/149), rate-
  limited to ~once per ~1s per channel (so we don't flood, but every ~1s of being on 149 the
  iPhone gets a fresh announce). This decouples discovery from luftlift's 10s cadence — the
  hop to 149 is no longer wasted.
- Keep it bounded + cheap; expire the cache if no new multicast arrives for a while (e.g. a
  few × the luftlift re-announce interval) so we stop advertising a dead receiver.
- The transfer pin (Bug C) still takes priority during an active unicast upload (don't
  re-broadcast announces while pinned mid-transfer).

### Part B: denser social-channel dwell (esp. 149)
Make filin spend more time on 149 so it overlaps the iPhone's 149 listen windows more often.
In ensure_social_coverage, after preserving the master's real channels, FILL the remaining
null/0 slots by round-robin of the social channels (6/44/149) instead of leaving them null.
Cap so the master's own channels are still well represented. Goal: 149 in several slots, not 1.

## Tests (TDD, keep green — filin at 103 tests)
- announce cache: a TAP multicast frame is cached; re-broadcast fires on a social-channel
  visit; rate-limit suppresses a second send within the window; a new TAP frame refreshes the
  cache; cache expires after the idle window.
- denser coverage: a master seq with many null slots yields a final seq with 149 in >1 slot
  and round-robin social fill; master's real channels still present.
- transfer pin still suppresses re-broadcast while a unicast transfer is active.
- existing multicast-spread / social-coverage / pin / hysteresis tests stay green.

## Constraints
- Edit ONLY crates/filin-rs. TDD, clippy -D warnings clean, release builds. Separate commit(s):
  `fix(filin): re-broadcast cached mDNS announce on each social-channel visit` and
  `fix(filin): denser social-channel dwell (fill null slots round-robin)`.
- Do NOT touch the running root filin pid (1806251), luftlift, workspace Cargo.toml, or
  owl/src/election.h.

When done, summarize what changed so the orchestrator can rebuild+restart and have the human
retry discovery (and watch /tmp/fF.log: 149 should appear in more slots + announces re-emit
~1s on 149).
