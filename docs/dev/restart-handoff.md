# RESTART-HANDOFF — rebuild the whole orchestration after a full reboot

**Why this file exists:** the carl9170 Wi-Fi card wedged at the USB level
(`usb 3-1: firmware upload failed (-110)` / `Failed to restart device (-110)`) and a
**full reboot of the Linux box (linux-host)** was required. That reboot kills EVERYTHING:
this Claude orchestrator (pty-1), both coding agents, the root shells, and all tttt PTYs.

**You are Claude, the orchestrator.** A human just restarted you in a fresh `claude` session
and pointed you here. Read this top-to-bottom, then ALSO read `handoff.md` (the running
project log — deep history + the per-heartbeat loop protocol). Then rebuild the environment
below and resume.

---

## 0. ONE-LINE STATE
Pure-software Rust port of owl(AWDL)+opendrop(AirDrop) is DONE, tested, and has worked live
(real Mac AirDropped 2 JPEGs cycle 28; iPhone did full Discover/Ask/Upload cycle 29). We were
mid-iteration on **live iPhone discovery** when the radio wedged. Code is committed and green;
the reboot is purely to un-wedge the radio. NOT starting over — resume from git HEAD.

## 1. HOST + PATHS
- Box: `linux-host`, login user `user` (you run as user). Repo:
  `/home/user/rust/opendrop` (git, branch `main`).
- Root shells are needed for hardware (filin/luftlift bind AF_PACKET + create awdl0 TAP =
  need root). They run in dedicated PTYs as `root@linux-host` (sudo).
- The MacBook is a SEPARATE machine (macOS 15.7.3), reachable in its own PTY; only needed for
  active Apple-side hardware tests. Its awdl0 MAC was aa:bb:cc:dd:ee:03.

## 2. PROJECT STATE (git HEAD at reboot = c6fec51)
- Workspace crates: `crates/filin-rs` (AWDL, bin `filin`), `crates/luftlift-rs` (AirDrop, bin
  `luftlift`), `crates/opendrop-rs` (thin stub). filin 113 tests, luftlift ~104 — all green,
  clippy clean.
- **Release binaries are ON DISK and current** (`target/release/filin`,
  `target/release/luftlift`) — NO rebuild needed unless you change code. (If in doubt:
  `cargo build --release -p filin-rs -p luftlift-rs`.)
- Uncommitted (intentional, leave as-is): `owl/src/election.h` (force-master patch, only for a
  faithful-owl baseline; NOT used — we run filin), a modified task `.md`, untracked `.claude/`.
  Do NOT `git add -A`; only add specific files.
- Recent fixes this session (cycles 30-37), all committed:
  - luftlift: iPhone /Upload decode (chunked-truncation + skip dir/"." CPIO entries),
    re-announce every 10s, raw upload dump via `LUFTLIFT_DUMP_UPLOAD` dir.
  - filin: chanseq decode→real channels, transfer-pin (hold peer channel during unicast
    upload), master-adoption hysteresis, social-channel coverage (6/44/149), and **re-broadcast
    the cached mDNS announce on each social-channel visit ~1s** + denser-149 dwell.
- **CURRENT OPEN TASK** = verify live iPhone DISCOVERY now works (luftlift appears in the
  iPhone picker) with the re-broadcast/149 fixes, then complete a real photo /Upload
  (non-empty file in /tmp/ldrop + raw capture in /tmp/ldrop_raw). See handoff.md cycle 37.

## 3. HARDWARE BRING-UP (do FIRST after reboot, before launching the stack)
The whole point of the reboot. In a ROOT shell:
1. Confirm the card enumerated cleanly (NO -110 this time):
   `dmesg | grep -iE 'carl9170|firmware' | tail` → want "firmware ... loaded", not "failed".
2. Confirm the monitor iface exists (name is MAC-derived, should be the same):
   `ip -br link show wlxbc05430d50a1`  (carl9170, MAC aa:bb:cc:dd:ee:01)
3. Put it in monitor mode on channel 44:
   ```
   nmcli dev set wlxbc05430d50a1 managed no
   ip link set wlxbc05430d50a1 down
   iw dev wlxbc05430d50a1 set type monitor
   ip link set wlxbc05430d50a1 up
   iw dev wlxbc05430d50a1 set channel 44
   ```
4. If firmware STILL fails (-110) after the reboot: try a different USB port; the card is an
   Atheros carl9170 (AR9170). Backup radio = Realtek `wlx90de80df7307` (rtw88, marginal). Per
   [[no-hardware-defeatism]] memory: this card IS proven-capable — don't write it off; recover it.

## 4. LAUNCH THE filin+luftlift STACK (root shell, once hardware is up)
Kill any stragglers first: `pkill -9 -f target/release/filin; pkill -9 -f target/release/luftlift`
Then (filin first, it creates awdl0; then luftlift):
```
cd /home/user/rust/opendrop
mkdir -p /tmp/ldrop /tmp/ldrop_raw
RUST_LOG=filin_rs=debug ./target/release/filin -i wlxbc05430d50a1 -c 44 -h awdl0 -N > /tmp/fF.log 2>&1 &
sleep 4
RUST_LOG=luftlift_rs=debug,luftlift=info LUFTLIFT_DUMP_UPLOAD=/tmp/ldrop_raw ./target/release/luftlift -i awdl0 receive -o /tmp/ldrop > /tmp/lrF.log 2>&1 &
```
Verify healthy:
- filin hops incl. 149: `sed 's/\x1b\[[0-9;]*m//g' /tmp/fF.log | grep -oE 'channel=[0-9]+' | sort | uniq -c`
- announce re-broadcast on 149: `grep 're-broadcast cached' /tmp/fF.log | grep -oE 'channel=[0-9]+' | sort | uniq -c`
- luftlift announcing + listener bound: `tail /tmp/lrF.log` (want "mDNS re-announcer started interval_secs=10" + "HTTPS listener bound port=8771")
- on-wire announce: `timeout 6 tcpdump -i awdl0 -nn -c4 udp port 5353`  (need root + raw socket;
  run in the ROOT PTY, NOT via your sandboxed Bash which lacks CAP_NET_RAW)
Then ask the human to open the iPhone AirDrop sheet → luftlift should appear → send a PHOTO.

## 5. RECREATE THE tttt ENVIRONMENT (PTYs / agents)
**IMPORTANT mental model:** the tttt PTYs are persistent terminal WINDOWS (shells). The
"agents" are just **commands you run INSIDE a window** — `opencode`, `codex`, `claude` — they
are NOT separate `tttt -e <agent>` instances. So to restore an agent: pick/identify a window
(`tttt_pty_list` + read each screen), and in it run the agent's command, then RE-BRIEF it
(fresh agents have NO memory — point them at the repo files). The human normally sets up the
windows; you drive what runs inside them via `tttt_pty_send_keys`.

**Always start a window as a plain `bash` shell, then run the agent command inside it** — do
NOT launch a window directly into `opencode`/`codex`/`claude`. That way the window persists as a
shell when the agent exits/crashes, so you can simply re-run the command (or run git/cargo/
hardware commands) in the same window instead of losing it. If you create windows via
`tttt_pty_launch`, launch `bash`; for the root hardware window, a root `bash` (sudo).

- **YOU = orchestrator**: the `claude` command running in your window. You read this file +
  handoff.md and drive. Heartbeat loop via **ScheduleWakeup** (~180-360s; prompt = "HEARTBEAT:
  read handoff.md and continue"). NEVER use self_inject for the loop.
- **filin agent** (owns ONLY `crates/filin-rs`): run `opencode` in a window (was GLM-5.2).
  Re-brief: "You own crates/filin-rs of a clean-room Rust AWDL port. Read ARCHITECTURE.md,
  WORKFLOW.md (TDD: red/green/refactor commits), crates/filin-rs/PORT.md, and the FILIN_*.md
  task/diagnosis files. The code is feature-complete + 113 tests; current work is live-discovery
  tuning (announce re-broadcast on social channels, ch149 coverage)." TDD, commit per change,
  only edits filin-rs. OpenCode runs commands autonomously; long prompts MANGLE on paste — write
  the task to a *.md and send a short "read X and do it".
- **luftlift agent** (owns ONLY `crates/luftlift-rs`): run `opencode` in a window (GLM-5.2).
  Re-brief: "You own crates/luftlift-rs (AirDrop). Read ARCHITECTURE.md, WORKFLOW.md,
  crates/luftlift-rs/PORT.md, docs/airdrop-protocol.md, the LUFTLIFT_*.md files, and the python
  reference opendrop-patch/. ~104 tests green; current state = receive works, decode fixed."
- (Optional) **Codex agent**: run `codex` in a window (gpt-5.5) — was STOOD DOWN off filin to
  avoid edit conflicts on the single working tree. Only ONE agent edits filin at a time. Leave
  idle unless you deliberately reassign. Codex is a TUI that asks per-command APPROVAL (send the
  text, then [ENTER] as a SEPARATE call; sometimes a 2nd [ENTER]; approve with [ENTER]).
- **Root hardware shell(s)**: a window with a `root@linux-host` shell (sudo) for the carl9170
  + stack (sections 3-4). SHORT commands only (long one-liners mangle). One is enough; a 2nd is
  handy.
- **MacBook shell** (optional, only for Apple-side tests): a window with a root shell on the
  MacBook (separate machine).
- **notify watcher** (only if you run a Codex agent that asks for approvals): create a
  `notify_on_pattern` watching the Codex window for `Would you like to run|Press enter to confirm
  or esc`, injecting a heartbeat into YOUR window so you wake on its approval prompts. Not needed
  for the OpenCode (autonomous) agents.

NOTE: PTY numbers are NOT guaranteed to be the same after a restart, and a window may be a plain
shell or already running an agent — ALWAYS identify each by reading its screen (`tttt_pty_list`
+ `tttt_pty_get_screen`), never assume the old numbering or contents.

## 6. THE LOOP + WHERE TO LOOK
- Per-heartbeat: `git -C /home/user/rust/opendrop log --oneline -15`; check each agent PTY
  (approve/answer/nudge); keep `cargo test` green; update + commit `handoff.md`; ScheduleWakeup;
  stop. (Full protocol in handoff.md.)
- Your Bash tool is SANDBOXED (no CAP_NET_RAW, can't kill root procs) — do tcpdump / pkill of
  root filin/luftlift / iw / ip-link in a ROOT PTY, not via Bash.
- Memory (persists across sessions): `[[no-hardware-defeatism]]` (the radio is capable — blame
  our software, recover the card; don't shelve), `[[awdl-wifi-hardware]]`, plus the heartbeat-loop
  memory.

## 7. FIRST ACTIONS AFTER YOU READ THIS
1. `tttt_pty_list` + read each PTY screen to see what the human already relaunched.
2. Bring up the carl9170 (§3) and verify firmware loaded cleanly (no -110).
3. Launch filin+luftlift (§4), verify 149 coverage + re-broadcast on-wire.
4. Re-brief any fresh agents (§5) if they need tasking; otherwise leave idle.
5. Tell the human it's ready and to retry the iPhone photo; watch /tmp/lrF.log for
   Discover/Ask/Upload and /tmp/ldrop for the received file.
6. Resume the handoff.md heartbeat loop.
