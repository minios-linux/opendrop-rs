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
