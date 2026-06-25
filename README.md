# opendrop-rs — AWDL + AirDrop for Linux, in Rust

A clean-room Rust port of [owl](https://github.com/seemoo-lab/owl) (Apple Wireless
Direct Link / AWDL) and [opendrop](https://github.com/seemoo-lab/opendrop) (AirDrop)
for Linux. The goal is a working, tested AirDrop send/receive stack on commodity
Linux Wi-Fi hardware, without depending on the original C/Python implementations.

The workspace is split into two independent binaries:

| Crate                | Binary     | Role |
|----------------------|------------|------|
| `crates/filin-rs`    | `filin`    | AWDL link layer — creates `awdl0`, drives a Wi-Fi monitor interface, runs election/sync, bridges Ethernet ↔ AWDL frames. (port of *owl*) |
| `crates/luftlift-rs` | `luftlift` | AirDrop application — mDNS discovery, TLS/HTTPS, the `/Discover` `/Ask` `/Upload` protocol; can **send**, **receive**, and **find** peers. (port of *opendrop*) |
| `crates/opendrop-rs` | —          | Reserved for an optional unified CLI wrapper; currently a stub. |

The names are trademark-safe riffs on the originals: **filin** (Russian "филин",
eagle-owl) for the owl layer, **luftlift** (German "air lift") for AirDrop.

## Status

The pure-software stack is **feature-complete, unit-tested, and clippy-clean**:

- `cargo build --release` — both binaries build clean.
- `cargo test --workspace` — 92 tests passing.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.

What is **not** yet done: live interop against a real Apple device. AWDL sync over a
USB monitor interface is timing-sensitive and needs real hardware (see
[Hardware](#hardware)). Everything feeding that path — the radiotap TSFT bridge,
AWDL parsers, election/sync/channel logic, and the OS-level TAP/AF_PACKET/nl80211
wrappers — is implemented and tested, but the end-to-end radio path has not been
validated live. Treat the first hardware run as bring-up, not regression.

## Architecture

```
       Wi-Fi monitor iface (raw 802.11, e.g. carl9170 @ ch44)
                         │
                   ┌─────┴─────┐
                   │  filin    │   radiotap → 802.11 → AWDL action/data frames
                   │ (AWDL)    │   election · sync · TSFT bridge · channel · TX
                   └─────┬─────┘
                         │  creates + bridges
                      awdl0 (TAP, MTU 1450)
                         │
                   ┌─────┴─────┐
                   │ luftlift  │   mDNS · TLS · HTTP · dvzip/CPIO · Discover/Ask/Upload
                   │ (AirDrop) │   send · receive · find
                   └───────────┘
```

`filin` brings up the AWDL link and exposes it as an ordinary `awdl0` network
interface; `luftlift` then speaks AirDrop over `awdl0` (or any interface) like any
other IPv6 link-local service. The two are decoupled — `luftlift find` works on a
normal LAN without `filin` running.

Design notes worth knowing:

- **AWDL sync uses the radiotap TSFT hardware timestamp**, bridged into monotonic
  time (offset filtered with clamping), *not* host packet-arrival time. This is the
  single most important correctness point for sync; see `docs/awdl-sync.md`.
- **AirDrop bodies are `application/x-dvzip`**: framed `[u32 BE len][zlib]` blocks
  wrapping an ODC CPIO archive (magic `070707`). POSTs are chunked (no
  `Content-Length`). See `docs/airdrop-protocol.md`.
- Built TDD: every feature landed as a `red` → `green` (→ `refactor`) commit triple.

## Build

```bash
cargo build --release
# binaries: target/release/filin, target/release/luftlift
cargo test --workspace      # 92 tests
```

## Usage

### luftlift (AirDrop) — works on a normal network today

```bash
# Discover AirDrop senders on the local link (no root, no filin needed)
luftlift find -d 5

# Receive files (announces via mDNS, runs the HTTPS server)
luftlift receive -o ./incoming

# Send a file to a discovered receiver (instance name from `find`)
luftlift send "<recipient-instance-name>" ./photo.jpg
```

Set `RUST_LOG` for detail, e.g. `RUST_LOG=luftlift=debug luftlift receive`.

> Note: real AirDrop interop with Apple devices requires AWDL to be up, which is
> what `filin` provides. `luftlift` against another `luftlift` works over any
> shared link.

### filin (AWDL) — requires root + Wi-Fi monitor hardware

```bash
sudo filin -i <monitor_iface> -c 44 -h awdl0 -N
```

| Flag | Meaning |
|------|---------|
| `-i <iface>` | Wi-Fi monitor interface for raw 802.11 RX/TX |
| `-c 44`      | anchor/social channel (44 for modern Apple; also 6, 149) |
| `-h awdl0`   | name of the TAP interface to create |
| `-N`         | assume the interface is already in monitor mode |

At startup `filin` creates the `awdl0` TAP, copies the monitor MAC, sets MTU 1450
and brings it up, sets the channel via nl80211, then enters the RX/TX poll loop.
Full operator guide — including how to verify sync and inspect `awdl0` traffic — is
in **[`crates/filin-rs/RUN.md`](crates/filin-rs/RUN.md)**.

## Bringing up AirDrop end-to-end

In two terminals on the Linux box:

```bash
# 1. AWDL link up
sudo filin -i <monitor_iface> -c 44 -h awdl0 -N

# 2. AirDrop receiver bound to the awdl0 link
luftlift receive -o ./incoming
```

Then, on an Apple device with the Sharing pane open, AirDrop a file to the Linux
host. Watch `filin`'s `sync_error` log line settle to a low, stable value (that's
the sign AWDL sync is usable) before expecting traffic on `awdl0`.

## Concrete run sequence after a reboot (this box: ayourtch-x280)

The exact commands to bring the live stack back up by hand. The carl9170 monitor
interface is `wlxbc05430d50a1` (MAC `bc:05:43:0d:50:a1`). Run everything below as
**root** (the filin/luftlift processes need raw sockets + TAP).

```bash
# 0. Verify the card enumerated and loaded firmware CLEANLY (no -110 errors).
dmesg | grep -iE 'carl9170|firmware' | tail
#   want a successful firmware load; if you see 'firmware upload failed (-110)' /
#   'Failed to restart device (-110)', the card is wedged: try another USB port
#   (a software re-auth `echo 0|1 > /sys/bus/usb/devices/<dev>/authorized` usually
#   does NOT recover a -110; a fresh replug into a different port, or reboot, does).

# 1. Put the card into plain monitor mode on channel 44.
nmcli dev set wlxbc05430d50a1 managed no
ip link set wlxbc05430d50a1 down
iw dev wlxbc05430d50a1 set type monitor
ip link set wlxbc05430d50a1 up
iw dev wlxbc05430d50a1 set channel 44
ip -br link show wlxbc05430d50a1          # expect: ... UP ...

# 2. Repo + output dirs.
cd /home/ayourtch/rust/opendrop
mkdir -p /tmp/ldrop /tmp/ldrop_raw
# (binaries are prebuilt in target/release; rebuild only if code changed:)
# cargo build --release -p filin-rs -p luftlift-rs

# 3. Kill any stragglers, then launch filin FIRST (it creates awdl0), then luftlift.
pkill -9 -f target/release/filin; pkill -9 -f target/release/luftlift; sleep 2
RUST_LOG=filin_rs=debug \
  ./target/release/filin -i wlxbc05430d50a1 -c 44 -h awdl0 -N > /tmp/fF.log 2>&1 &
sleep 4
RUST_LOG=luftlift_rs=debug,luftlift=info LUFTLIFT_DUMP_UPLOAD=/tmp/ldrop_raw \
  ./target/release/luftlift -i awdl0 receive -o /tmp/ldrop > /tmp/lrF.log 2>&1 &
sleep 4
pgrep -af 'target/release/(filin|luftlift)'   # both should be listed
```

Verify it's healthy (strip color codes with the `sed` below):

```bash
# filin is hopping and DOES visit ch149 (the AirDrop social channel):
sed 's/\x1b\[[0-9;]*m//g' /tmp/fF.log | grep -oE 'channel=[0-9]+' | sort | uniq -c
# the cached mDNS announce is re-broadcast ~1s on the social channels incl 149:
sed 's/\x1b\[[0-9;]*m//g' /tmp/fF.log | grep 're-broadcast cached' \
  | grep -oE 'channel=[0-9]+' | sort | uniq -c
# luftlift announced + HTTPS listener bound + re-announcing every 10s:
tail /tmp/lrF.log
# announce is actually on-wire (run as root; needs CAP_NET_RAW):
timeout 6 tcpdump -i awdl0 -nn -c4 udp port 5353
```

Now open the AirDrop sheet on the iPhone → `luftlift` should appear in the picker →
send a **photo**. Watch the result:

```bash
# discovery + transfer progress:
sed 's/\x1b\[[0-9;]*m//g' /tmp/lrF.log | grep -iE 'Discover|Ask|Upload|received|error'
# the received file (should be non-empty) and the raw dvzip capture for offline diff:
ls -l /tmp/ldrop/ /tmp/ldrop_raw/
# cross-check a raw capture against the python reference decoder:
python3 opendrop-patch/dvzip.py /tmp/ldrop_raw/upload-*.bin   # -> *.cpio
```

To restart just the stack later: `pkill -9 -f target/release/filin; pkill -9 -f
target/release/luftlift`, then re-run step 3 (filin first, then luftlift).

## Hardware

AWDL sync over USB monitor interfaces is the flaky part and is best done with the
human present.

- **Known-good:** Atheros `carl9170` USB in plain monitor mode on channel 44.
- Realtek `rtw88` (patched) works but is marginal.
- Use **plain** monitor mode; do not rely on active monitor mode.

See project notes for more on which radios can inject AWDL frames reliably.

## Repository layout

```
crates/filin-rs/      AWDL link layer (bin: filin)   + RUN.md operator guide
crates/luftlift-rs/   AirDrop app    (bin: luftlift)
crates/opendrop-rs/   reserved unified-CLI stub
docs/awdl-sync.md     the TSFT-timestamp sync lesson
docs/airdrop-protocol.md  reverse-engineered iOS AirDrop wire details
```

## Credits & license

Clean-room port inspired by the SEEMOO Lab `owl` and `opendrop` projects and the
AWDL/AirDrop reverse-engineering literature. Dual-licensed under MIT or Apache-2.0.
