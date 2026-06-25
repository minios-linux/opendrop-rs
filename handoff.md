# HANDOFF — orchestrator context for the filin/luftlift Rust port

You are **Claude**, the orchestrator of a multi-hour autonomous task. Your context
was just self-cleared to stay lean. This file is your memory. Read it fully, then
continue. **Keep this file updated every cycle** — it is the single source of truth.

## MISSION
Drive two other AI coding agents to build a clean-room **Rust port of owl (AWDL)
+ opendrop (AirDrop)** for Linux. The human (ayourtch) stepped away and wants to
return in a few hours to a *working, tested implementation* without having to read
the C/Python references himself. You do the integration/review; the agents code.

Honest target: the pure-software port (parsers, codecs, protocol, mDNS, TLS, HTTP)
fully implemented + unit-tested + integrated + compiling is achievable. Live
hardware-validated interop with a real Apple device is the one thing that may need
a final hands-on pass with the human present (flaky AWDL-sync-on-USB; needs the
Atheros card + an awake iPhone). Flag that at the end; don't block on it.

## THE SELF-CLEAR LOOP (keep doing this — it's how you survive multi-hour run)
Every cycle:
1. Review both agents (pty-2, pty-3): read screens, approve pending Codex prompts,
   review new commits (`cd /home/ayourtch/rust/opendrop && git log --oneline -15`),
   unblock anyone who's stuck/asking, nudge anyone idle to continue their PORT.md.
2. Update THIS file (handoff.md) with the new state (commits done, milestone, any
   blockers/decisions). Commit it: `git add handoff.md && git commit -m "chore(handoff): <state>"`.
3. Schedule the next heartbeat with **ScheduleWakeup** (delaySeconds ~180), prompt =
   "HEARTBEAT: read /home/ayourtch/rust/opendrop/handoff.md fully and continue
   orchestrating per it." Then STOP — the wakeup re-fires you fresh, which both
   clears stale context AND reliably makes you re-read this file.
   **DO NOT use `mcp__tttt__tttt_self_inject` for the self-clear.** It dumps text
   into your own stdin without re-reading handoff.md and the submit/enter gets
   dropped — the ScheduleWakeup internal heartbeat is the correct, only mechanism.
Tune the interval up (e.g. 300-600s) if agents need more time per milestone; down
if Codex is blocking on approvals. Cache cost is irrelevant across heartbeats.

**Event-based wake for pty-2 approvals:** a `notify_on_pattern` watcher (`notify-1`,
watching pty-2 for `Would you like to run|Press enter to confirm or esc`) injects a
heartbeat into pty-1 the moment Codex blocks on an approval — so you don't idle the
full ~180s while pty-2 waits. Check it's still registered with
`mcp__tttt__tttt_notify_list`; re-create it (see the notify_on_pattern call) if gone.
This is the ONE sanctioned inject into pty-1 — it's event-triggered + auto-submits,
unlike the broken self_inject self-clear.

## THE AGENTS & TERMINALS (tttt PTYs)
- **pty-1** = YOU (claude, root). Self-inject /clear here.
- **pty-2** = **gpt-5.5** via OpenAI Codex CLI → owns crate **`crates/filin-rs/`**
  (the AWDL/owl port). DRIVING QUIRKS: it's a TUI. To send a prompt: send the text,
  then [ENTER] as a SEPARATE call; sometimes needs a 2ND [ENTER] (it shows "[Pasted
  Content N chars]" then expands; press Enter again to submit). It asks for
  per-command APPROVAL ("Press enter to confirm or esc to cancel" / options 1/2/3) —
  approve with [ENTER] (option 1); use option 2 ("don't ask again for commands
  starting with…") to reduce future prompts. Check it every heartbeat or it blocks.
- **pty-3** = **GLM-5.2** (OpenCode CLI) → owns crate **`crates/luftlift-rs/`**
  (the AirDrop/opendrop port). Runs commands AUTONOMOUSLY (no approval prompts).
  To send a prompt: text then [ENTER]. Reports "paused"/status; nudge it to continue
  if it stops between milestones.
- **pty-7, pty-8** = root shells on the Linux box (for hardware/owl/opendrop testing
  if you get to hardware bring-up). SHORT commands only (long one-liners get mangled).
- **pty-9** = root shell on the MacBook (macOS 15.7.3, awdl0 MAC 5e:db:75:dd:17:2e —
  it's the neighbor AWDL master we kept syncing to). Only relevant for hardware tests.

Read a PTY: `mcp__tttt__tttt_pty_get_screen(session_id)`. Wait: `..._wait_for_idle`.
Send: `..._pty_send_keys(session_id, keys)` then a separate `[ENTER]`.

## REPO + CONVENTIONS
- Repo: `/home/ayourtch/rust/opendrop` (git, branch `main`, ONE working tree).
- Cargo workspace. Crates: `crates/filin-rs` (gpt-5.5), `crates/luftlift-rs` (GLM-5.2),
  `crates/opendrop-rs` (yours — orchestrator CLI, mostly empty for now).
- **Subdirectory ownership**: each agent edits ONLY its crate. You own the workspace
  `Cargo.toml`, `docs/`, `crates/opendrop-rs/`, and integration. If an agent needs a
  shared dep, hoist it into `[workspace.dependencies]` yourself.
- **TDD with staged commits**: `red(<crate>): …`, `green(<crate>): …`,
  `refactor(<crate>): …`. crate = `filin` or `luftlift`. Verify `cargo test` green.
- Naming (trademark-safe, do NOT revert): crate `filin-rs` (бин `filin`) = "eagle-owl"
  (Russian, riffs on upstream owl). crate `luftlift-rs` (bin `luftlift`) = "air-lift"
  (German). Never name things `awdl-rs`/`airdrop-rs`.

## SPECS & GROUND TRUTH (the agents have these; you point/enforce)
- `ARCHITECTURE.md`, `WORKFLOW.md` — overall design + TDD rules.
- `crates/filin-rs/PORT.md` — filin milestones (radiotap→802.11→AWDL frames→election→
  sync→channel→TX→awdl0).
- `crates/luftlift-rs/PORT.md` — luftlift milestones (dvzip→CPIO→chunked body→plist→
  mDNS/TLS/HTTP handlers→receive).
- `docs/awdl-sync.md` — THE critical lesson: AWDL sync must use radiotap **TSFT
  hardware timestamp**, not host clock (bridge TSF→monotonic via EMA+clamp offset).
- `docs/airdrop-protocol.md` — reverse-engineered iOS-18 details: chunked POSTs (no
  Content-Length), `application/x-dvzip` = framed `[u32 BE len][zlib]`→ODC CPIO
  (magic 070707), links are .webloc files. Reference decoder: `opendrop-patch/dvzip.py`.
- References: `owl/` (C), `opendrop-patch/` (patched Python + dvzip.py), `rtw88-patch/`.
- Deeper hardware/protocol findings are in project memory (awdl-wifi-hardware.md).

## CURRENT STATE (UPDATE THIS EACH CYCLE)
Last updated: cycle 15 (LIVE HARDWARE PASS in progress with human — carl9170 + real Apple peer).
*** LIVE VALIDATION (human present, driving pty-7 root shell on ayourtch-x280) ***
- HARDWARE: monitor iface = wlxbc05430d50a1 (carl9170, monitor mode ch44). awdl0
  created by filin (or C-owl). Real Apple peer (iPad/Mac "Andrew's MacBook Pro").
- ✅ luftlift (AirDrop) FULLY VALIDATED end-to-end over a C-owl AWDL link: received a
  real photo IMG_0226.png (568137 B, valid PNG) via Discover->Ask->Upload. Found+fixed
  a real interop bug live: /Ask has NO TransferID from real iOS -> made it optional
  (commit 3539f53 fix(luftlift): accept /Ask without TransferID). Link/webloc sends
  come as Ask file_count=0 + no Upload = KNOWN follow-up (not done).
- 🔧 filin (our AWDL layer) live debugging (Stage 2, swap C-owl -> filin):
  - awdl0 TAP create + MAC + MTU + IPv6 link-local + TX inject all WORK.
  - BUG1 FIXED: radiotap parser only knew present bits 0/1/5, errored on real carl9170
    headers (bits 1,2,3,5,11,14, no TSFT). gpt-5.5 fixed: skip-table + TSFT host-clock
    fallback (commits red/green parse carl9170 radiotap + tsft fallback + tolerate
    unknown fields). 1129->0 radiotap errors live.
  - BUG2 IN PROGRESS: real AWDL action frames (d0, Apple OUI 00:17:f2, ~5000 of them)
    ALL rejected as Awdl(TooShort). RX buf is 4096 (lib.rs:1388) so NOT truncation —
    real frames reach awdl::parse_action_frame (lib.rs:400, needs body>=16) but the
    body slice arrives <16, so the 802.11/AWDL body OFFSET is wrong. Root cause is in
    the runtime RX slicing (radiotap->802.11->action body), diverging from owl/src/rx.c.
  - DELEGATION (human-approved): GLM in pty-5 (fresh OpenCode) owns this fix now —
    comparing owl/src/rx.c + frame.h vs filin, writing diagnosis to repo-root
    FILIN_RX_DIAGNOSIS.md, then implementing the fix (TDD) + cargo build --release.
    Read the FILE (not terminal) for its report. gpt-5.5 (pty-2) STAYS OFF filin to
    avoid edit conflicts. gpt-5.5 already did radiotap fixes (commits up to 0b28a43).
  - NOTE: gpt-5.5 hit usage limit once (~02:18), human reset it. Watch for re-limit.
  - owl reference for AWDL action parse: rx.c awdl_parse_action_hdr (~L234):
    category=0x7f, oui=00:17:f2, version=AWDL_VERSION_COMPAT(=0x10), subtype PSF=0/MIF=3;
    struct awdl_action = category(1)+oui(3)+type(1)+version(1)+subtype(1).
- Live test recipe: kill owl/luftlift/filin; filin -i wlxbc05430d50a1 -c 44 -h awdl0 -N;
  then luftlift -i awdl0 receive -o /tmp/ldrop; human AirDrops a file; watch
  /tmp/ldrop + luftlift log for Discover/Ask/Upload, filin trace for sync_error.
--- earlier ---
Last updated: cycle 14 (DONE — pure-software port complete, verified, reported to human).
*** PRIOR: pure-software port complete, verified, reported. ***
- FINAL VERIFICATION (all green):
  - cargo build --release: clean, both bins (target/release/filin, .../luftlift).
  - cargo test --workspace: 92 passing (filin 28 + luftlift 64).
  - cargo clippy --workspace --all-targets -- -D warnings: clean.
  - filin --help works (accurate startup log, --help restored, -h=host iface).
  - luftlift --help (receive/find/send) + `luftlift find -d N` clean output (NSEC
    spam silenced).
- Both agents reported feature-complete and are standing by. Polish fixes all landed
  (filin 8070a38 help/log; luftlift 35543ad find-noise; filin 93e35b7 clippy).
- Reported success to the human this cycle. The ONE remaining item = live hardware
  interop pass (carl9170 monitor ch44 + awake iPhone); steps in crates/filin-rs/RUN.md.
- opendrop-rs = 16-line stub (optional wrapper; the two crate bins are the deliverable).
- IF the human returns and wants more: (a) live hardware bring-up w/ filin RUN.md;
  (b) luftlift real-fixture test once /tmp/upload.bin exists; (c) opendrop-rs unified
  CLI wrapper; (d) TLS integration test w/ threaded harness. None are blockers.
--- (prior cycle notes below) ---
Last updated: cycle 13 (ENDGAME: both feature-complete; release build+91 tests+clippy GREEN; 3 polish fixes in flight).
- BOTH CRATES FEATURE-COMPLETE. luftlift send landed (real TLS transport + main wiring),
  full peer = send+receive+find. filin done since cycle 11.
- Integration pass RESULTS:
  - `cargo build --release` = clean (both bins).
  - `cargo test --workspace` = 91 green (filin 27 + luftlift 64).
  - `cargo clippy --workspace --all-targets -- -D warnings` = clean (after filin fixed
    2 lints: slice::from_ref + items-after-test-module, commit 93e35b7).
  - `luftlift --help` clean (receive/find/send subcommands); `luftlift find -d 3` works.
- 3 POLISH FIXES DISPATCHED (verify landed next heartbeat, then write report):
  1. filin: restore `--help` long flag (was killed by disable_help_flag for -h host) +
     fix stale "skeleton — see PORT.md" startup log. (pty-2)
  2. luftlift: silence mdns_sd NSEC parse-error spam flooding `find` output (EnvFilter
     mdns_sd::service_daemon=off). (pty-3)
- NEXT: confirm those 3 land + re-run build/test/clippy + re-check `filin --help` and
  `luftlift find`; then WRITE THE SUCCESS REPORT to the human (this is the surface-to-
  human moment per the protocol). Mention opendrop-rs is an empty stub (the two crate
  bins are the deliverable). Live hardware interop = the one remaining manual pass.
--- (prior cycle notes below) ---
Last updated: cycle 12 (91 tests green; filin DONE; luftlift on its LAST milestone — send TLS+main).
- filin: COMPLETE (27 tests, clippy clean, RUN.md done). Standing by. No more work.
- luftlift: send client nearly done — CPIO/dvzip encoders, TLS client cfg, request
  builders + send flow (w/ UTI type map) all green. LAST item in flight: RustlsTransport
  (real TLS stream) + wire `send` into main.rs. 64 tests. Then it's a full peer
  (send+receive+find).
- Tree: 91 tests green (filin 27 + luftlift 64).
- *** NEXT HEARTBEAT = likely ENDGAME ***: once luftlift's send+main lands (watch for
  the green commit + idle), RUN THE INTEGRATION PASS:
    1. cd /home/ayourtch/rust/opendrop
    2. cargo build --release            (both bins compile in release)
    3. cargo test --workspace           (all green)
    4. cargo clippy --workspace --all-targets -- -D warnings
    5. ./target/release/filin --help ; ./target/release/luftlift --help
       (and `luftlift find -d 3` is safe/verified; do NOT run receive/awdl0 — no hw)
  Then write the SUCCESS REPORT to the human (see WHEN TO SURFACE): what works +
  tested (parsers/codecs/protocol/mDNS/TLS/HTTP, AWDL election/sync/TSFT bridge/
  channel/TX, AirDrop send+receive+find), how to run each bin, and the ONE remaining
  thing = live hardware interop pass (carl9170 monitor ch44 + awake iPhone; filin
  RUN.md has the steps). Don't claim live-validated — it's compile+unit-tested.
--- (prior cycle notes below) ---
Last updated: cycle 11 (filin COMPLETE incl OS layer; luftlift building send client; 79 tests green).
- filin (gpt-5.5): **DONE through all of PORT.md incl. the real OS layer** — TAP awdl0,
  AF_PACKET RX/TX, netdev ioctls (MAC/MTU1450/up), nl80211 channel set, binary poll
  loop, TAP<->AWDL inject/decap, TSFT-bridged sync-error logging. 27 tests, clippy
  clean, git clean. Idle. Tasked it with one finisher: crates/filin-rs/RUN.md operator
  guide for the hardware pass (then stand by). filin needs NO more feature work.
- luftlift (GLM-5.2): building `send` client (TDD) — sender HTTP done; doing CPIO
  encoder -> dvzip encoder -> TLS client cfg -> request builders -> send orchestration
  -> main wiring. 52 tests. Autonomous.
- Tree: 79 tests green (filin 27 + luftlift 52), both clippy clean.
- ENDGAME now mostly gated on luftlift `send`. When it lands: root `cargo build
  --release` + `cargo test --workspace` + `cargo clippy --workspace -- -D warnings`,
  run `filin --help` / `luftlift --help` / `luftlift find` sanity, then SUCCESS REPORT
  to human (what works + tested, how to run each bin, what needs the hardware pass).
  opendrop-rs is still a near-empty stub — the two crate bins (filin, luftlift) are the
  deliverables; opendrop-rs wrapper is optional, don't block on it.
--- (prior cycle notes below) ---
Last updated: cycle 10 (luftlift RECEIVE+FIND complete; filin awdl0 bringup wired; both finishing).
- luftlift (GLM-5.2): FEATURE-COMPLETE receive path — tls/mdns(announce+browse)/
  server(serve_forever + tested handle_one_connection)/netutil(/proc/net/if_inet6)/
  main CLI (receive -o | find -d | send). `find` VERIFIED working (browses 5s, prints
  peers). All deps crate-local (flate2, plist, rcgen 0.13, rustls 0.23 ring,
  rustls-pemfile, mdns-sd). I nudged it to implement the `send` CLIENT (TLS client +
  POST /Discover//Ask//Upload as x-dvzip) so it's a full peer like opendrop — TDD,
  autonomous. Known non-blockers it flagged: real-fixture test needs /tmp/upload.bin
  (absent here); TLS integration test needs threaded harness; receive needs real
  awdl0+IPv6 (hardware pass).
- filin (gpt-5.5): awdl0 bringup fully wired into binary — nl80211 channel set, link
  bringup, anchor channel, link poll loop; now Ethernet<->AWDL data encap (TX). Tests
  25 pass + the usual in-flight RED. Self-committing via auto-reviewer.
- ENDGAME getting close: when filin's TX encap + RX/TX loop land and luftlift's send
  lands, do the full root build/test/clippy pass + binary --help sanity + success
  report (run instr + awdl0/hardware-pass caveat). opendrop-rs CLI still ~empty —
  decide if it needs a thin wrapper or if the two crate bins suffice (likely suffice).
--- (prior cycle notes below) ---
Last updated: cycle 9 (TREE GREEN 69 tests; filin syscall layer in TDD; luftlift main wiring).
- filin (24 tests): syscall layer underway w/ good discipline — tap-iface-request,
  packet-socket-bind, netdev-ioctl-name all have TESTED planning fns + thin unsafe
  runtime glue (compile-tested). Adding MAC/MTU/flags ioctl helpers next, then the
  actual RX/TX loop + awdl0 creation. NOTE: Codex auto-reviewer now auto-approves
  narrow filin TDD commits (standing authorization) -> fewer pty-2 approval prompts;
  watcher still armed for real questions/blockers.
- luftlift (45 tests, clippy clean): netutil + main.rs receive flow, then find (mDNS
  browse). Near done.
--- (prior cycle notes below) ---
Last updated: cycle 8 (TREE GREEN 62 tests; filin unblocked into HW syscalls; luftlift ~done).
- ***libc = "0.2" added to [workspace.dependencies]*** (commit) to unblock filin's
  tun/AF_PACKET/netlink work. filin uses `libc = { workspace = true }` in its crate.
- filin (gpt-5.5): ALL pure-software done — RX/TX encode+decode, election, sync, TSFT
  bridge, channel, CLI, full injectable 802.11 action frames. 21 tests pass, clippy
  -D warnings clean. Now implementing the REAL awdl0 bring-up (/dev/net/tun + AF_PACKET
  RX/TX + ioctl/netlink) — told to keep raw syscalls in a thin commented unsafe layer
  w/ testable logic factored out. This is the hardware-touching code; compiles != live-
  validated (still needs the carl9170 + iPhone pass with the human).
- luftlift (GLM-5.2): chunked/plist/HTTP/handlers/TLS/mDNS-announce/HTTPS-accept-loop
  ALL green (41 tests). Now: netutil (iface IPv6 link-local resolution) + main.rs
  receive-flow wiring, then sender-side `find` (mDNS browse). Autonomous, no blocker.
- Tree: `cargo test --workspace` = 62 passing, 0 fail. Endgame is close.
- Housekeeping: `.claude/` is untracked (orchestrator/notify cfg) — leave it; don't
  `git add -A`, only add specific files so agents' WIP/.claude don't get co-committed.
--- (prior cycle notes below) ---
Last updated: cycle 7 (filin binary wired w/ honest hw TODO; luftlift on server loop).
- filin (gpt-5.5): + integrated RX (TSFT-required), AWDL data->Ethernet decap, clap
  CLI (-i/-c/-h/-N), binary uses validated config. The ONE remaining gap is the
  hardware-touching bit, honestly stubbed: `TODO(gpt-5.5): open monitor iface and
  create awdl0`. That netlink/rawsock + awdl0 TUN bring-up is THE part for the live
  hardware pass with the human (carl9170 monitor, ch44) — everything feeding it is
  tested. Don't block on it.
- luftlift (GLM-5.2): building handle_one_connection (pure fn: reader->TLS/HTTP/
  handlers->writer) + accept loop = the receive flow. Then wire into main.rs. Close
  to feature-complete. Autonomous, no blocker.
- ENDGAME (when luftlift's server loop + main wiring land): root `cargo build` +
  `cargo test --workspace` + `cargo clippy`; sanity-run both binaries' --help/-N
  dry path; then write success report (run instructions + the awdl0/hw-pass caveat).
--- (prior cycle notes below) ---
Last updated: cycle 6 (filin ALL PORT.md milestones DONE + integrating; luftlift on mDNS).
- filin (gpt-5.5): COMPLETE through PORT.md — parsers, election, sync, TSFT bridge,
  sync-error, channel freq/slot, TX backpressure queue, iface config (awdl0/mon/ch44)
  all green. Now writing INTEGRATED RX tests (radiotap+802.11+AWDL MIF end-to-end,
  TSFT timestamp + RSSI carry-through). Essentially feature-complete; verifying.
- luftlift (GLM-5.2): dvzip, CPIO, chunked, plist, HTTP parser, handlers, **TLS green**
  (543ccd7). Now on mDNS announce (mdns-sd ServiceInfo API), then wire receive flow
  in main.rs. ~2 milestones from feature-complete. Autonomous, no blocker.
- APPROACHING TARGET: once luftlift finishes mDNS + receive-loop wiring, do a full
  workspace integration pass (cargo build/test/clippy at root), wire opendrop-rs CLI
  if needed, then write the success report (how to run + hardware-pass caveat).
--- (prior cycle notes below) ---
Last updated: cycle 5 (filin channel logic DONE; luftlift thrashing rustls 0.23 API — watch).
- filin (gpt-5.5): + sync-error compute, channel<->frequency, channel-slot-for-EAW
  (with %16 wraparound) all green. Stack complete through CHANNEL. Remaining per
  PORT.md: TX (frame build/serialize) + awdl0 interface bring-up.
- luftlift (GLM-5.2): TLS cert-gen — **self-resolving the rustls 0.23 / rcgen 0.13
  churn on its own** (switched to CertificateParams::new(..).self_signed(&key_pair)
  and PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(serialize_der)) to dodge the
  Clone issue). No injection needed — letting it self-resolve worked. If it regresses,
  the hint was: drop .with_safe_defaults() (gone in rustls 0.23); use
  .with_no_client_auth().with_single_cert(vec![cert_der], key_der)?.
--- (prior cycle notes below) ---
Last updated: cycle 4 (filin sync+TSFT bridge DONE; luftlift on TLS).
- filin (gpt-5.5): election (metric + tie-break) green; sync-update-from-hw-timestamp
  green; **TSFT bridge green** (the critical jitter-rejection / TSF->derived offset —
  matches docs/awdl-sync.md). Next: sync error calc + channel slot logic, then TX/awdl0.
- luftlift (GLM-5.2): /Discover //Ask /Upload handlers green. Now on M5 TLS cert gen
  (rcgen 0.13 generate_simple_self_signed -> rustls ServerConfig), then server loop,
  mDNS announce (mdns-sd), wire receive flow in main.rs. Adding rcgen/rustls to its
  OWN crate Cargo.toml — autonomous, no blocker. If it asks for a SHARED dep, hoist
  to [workspace.dependencies].
- Watch next: luftlift compiling against rcgen 0.13 (API churn risk) — if it thrashes
  on the rcgen API >2 cycles, that's a real blocker worth a nudge with the exact API.
--- (prior cycle notes below) ---
Last updated: cycle 3 (election algo + airdrop handlers underway).
- filin (gpt-5.5): all AWDL TLV parsers (action hdr, TLVs, sync-params, election-v2,
  channel-seq) green. Now into the ELECTION ALGORITHM ('elect higher metric master'
  green); compare_master by (counter, metric). Continuing through election/sync.
- luftlift (GLM-5.2): M3 chunked + M4 plist + M5 HTTP request parser all green. Now
  on M5 handlers (/Discover /Ask /Upload), then TLS(rcgen)+server loop, mDNS, wire
  receive flow in main.rs. Was iterating clippy -D warnings cleanup (lints, not real
  compile errors) — self-resolving, no blocker.
- NOTE on tree health: a workspace `cargo test` will often show a transient RED while
  an agent has written its next red test but not yet the green — and luftlift may
  briefly not compile under -D warnings mid-edit. Only intervene if a crate STAYS
  broken across cycles; otherwise let the owning agent finish its red/green pair.
--- (prior cycle notes below) ---
Last updated: cycle 2 (notify-1 watcher woke me on a pty-2 approval; both agents flying).
- filin (gpt-5.5): action header + AWDL TLVs + sync-params TLV all green; now on
  election-v2 TLV (RED just committed). Steady RED/green pairs, no blockers.
- luftlift (GLM-5.2): M3 chunked-body + M4 plist Discover/Ask done (green). Working
  M4 GREEN plist handlers + M5 (HTTP request parser, TLS rcgen, mDNS, handlers,
  receive loop) per its own todo list. Autonomous, no blockers.
- Tree carries a transient RED failure whenever an agent just committed a red stage;
  that's expected — confirm the matching green lands next cycle.
--- (prior cycle notes below) ---
Last updated: cycle 1 (fixed loop protocol: ScheduleWakeup, NOT self_inject).
- git log so far (newest first):
  - `82ec662 chore(handoff): orchestrator self-clear context (cycle 0)`
  - `529eea6 green(filin): parse radiotap header metadata` (gpt-5.5)
  - `ee15bb2 green(luftlift): odc cpio entry reader` (GLM)
  - `fe338ce red(luftlift): odc cpio entry reader`
  - `0f922b6 rename(crates): awdl-rs -> filin-rs, airdrop-rs -> luftlift-rs`
  - `3a5acbf red(awdl): parse radiotap header metadata`
  - `fb2ec41 green(airdrop): dvzip framed-zlib round-trip`
  - `c4d24db red(airdrop): dvzip framed-zlib round-trip` + scaffold/workflow.
- gpt-5.5 (filin): radiotap done (green). Wrote+committed `red(filin): parse 80211
  mac header` this cycle; now working the AWDL PSF vendor-action-frame parser
  (parse_action_frame, ActionSubtype::Psf, phy_tx/target_tx/TLVs). Continuing
  autonomously through filin PORT.md.
- GLM-5.2 (luftlift): dvzip + ODC CPIO reader done (green). Working autonomously on
  milestones 3→5: chunked HTTP body decode + minimal HTTP/1.1 request parser +
  plist + mDNS/TLS handlers. Read server.py.patched for grounding.
- Tree status: `cargo test` shows 1 expected RED failure (filin 802.11 header test,
  just committed as the red stage); gpt-5.5 is implementing its green now.
- Both on AUTONOMOUS directive (only stop on real blocker: workspace dep [name it],
  cross-crate contract, design fork).

## YOUR PER-HEARTBEAT CHECKLIST
1. `git -C /home/ayourtch/rust/opendrop log --oneline -15` — see new commits; sanity
   that they're stage-pure (red=test only, green=impl) and inside the right crate.
2. `cargo build` and `cargo test` at repo root — keep the tree green; if an agent
   broke the build, tell that agent to fix it (don't fix their crate yourself).
3. Read pty-2 screen: approve any pending command; if it asked you a blocker question,
   answer it (e.g., add the dep to workspace Cargo.toml, then tell it to proceed); if
   idle, nudge "continue your next PORT.md milestone, TDD + commits."
4. Read pty-3 screen: if blocked/asking, answer; if idle, nudge likewise.
5. If an agent needs a shared dep: add it to `[workspace.dependencies]` in the
   workspace Cargo.toml yourself, commit `chore(deps): add X`, tell the agent.
6. Update + commit this handoff.md, ScheduleWakeup ~180s, self-inject /clear to pty-1.

## WHEN TO SURFACE TO THE HUMAN (otherwise stay autonomous)
- A genuine design fork that needs his call.
- You've reached a working, tested implementation (report success + how to run +
  what still needs a hardware pass).
- A hard blocker you can't resolve (e.g., an agent is stuck in a loop, a dep won't
  build, or you need hardware he must set up).
Otherwise: keep driving, keep the loop, don't wait for him.

## HARDWARE (only if/when you reach integration & want live validation — optional, late)
- Working radio = **Atheros carl9170** USB, iface `wlxbc05430d50a1` on the Linux box
  (gives ~0.4% AWDL sync). Run owl-equivalent in plain monitor mode, channel 44.
- Realtek `wlx90de80df7307` (rtw88, our patched driver) = backup, marginal/crashy.
- This is the slow/flaky part; do it last and with the human ideally present.
