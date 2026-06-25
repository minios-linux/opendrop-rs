# rtw88-patch — stability fix for RTL8822BU under AWDL injection

Part of the opendrop-rs project. This is a patched build of the mainline Linux
`rtw88` WiFi driver that stops a Realtek **RTL8822BU** USB adapter from crashing
off the USB bus when running the AWDL daemon [`owl`](https://github.com/seemoo-lab/owl)
(which injects a high rate of 802.11 action frames through a monitor interface).

## The bug

owl's injected monitor frames carry `IEEE80211_TX_CTL_REQ_TX_STATUS`. The stock
`rtw88` driver responds by setting the **SPE_RPT** bit in the TX descriptor —
asking the firmware for a per-frame CCX TX report — and queuing the skb to wait
for it (`rtw_tx_report_enqueue`). In monitor mode that report never comes back,
so a 500 ms purge timer fires:

```
rtw_8822bu: failed to get tx report from firmware
```

Under owl's injection rate this floods the firmware's report path; the 8822BU
firmware then wedges and the **USB device disconnects off the bus**, requiring a
replug. (Same symptom as lwfinger/rtw88 issue #368.)

## The fix

One hunk in `tx.c` (`rtw_tx_pkt_info_update`): clear `REQ_TX_STATUS` for
vif-less (injected) / monitor frames *before* the report-enable check. Both the
SPE_RPT descriptor bit (`tx.c`) and the `usb.c` enqueue/purge path gate on this
flag, so clearing it once disables the whole tx-report machinery for these
frames. See `0001-rtw88-suppress-tx-report-for-monitor-injection.patch`.

Normal managed-mode operation is unaffected (those frames have a non-monitor
vif and keep their TX-status reporting).

## Result

On an RTL8822BU clone (`SerialNumber: 123456`) running `owl -N -c 44`:

| | stock rtw88 | patched rtw88 |
|---|---|---|
| `failed to get tx report` | spams every ~7 s | **0** |
| USB stability under owl | crashes off bus in ~1–2 min | **3+ min, no drop** |

This turns the adapter into a usable (if not ideal) AWDL dev platform. Note: it
does **not** fix AWDL time-sync precision (a separate problem — see the owl
`src/rx.c` hardware-TSF patch); reliable Apple discovery/transfer may still want
an `ath9k` card.

## Source

Mainline Linux **v6.8** `drivers/net/wireless/realtek/rtw88/`, built out-of-tree
against the running kernel's headers. Secure Boot must be off (unsigned modules).

## Build & load

```bash
./build.sh                 # builds rtw88_{core,usb,8822b,8822bu}.ko
sudo ./load.sh             # swaps stock stack for the patched modules
```

`load.sh` prints the loaded vs built `srcversion`; they should match. To revert,
just `modprobe -r` the stack and `modprobe rtw88_8822bu` to get the stock driver
back (or reboot).
