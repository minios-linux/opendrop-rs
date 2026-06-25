#!/bin/bash
# Swap the stock rtw88 stack for the locally-built patched modules.
# Run as root. Stop owl and free the interface first.
set -e
cd "$(dirname "$0")"
IF=${1:-wlx90de80df7307}

pkill -9 -f daemon/owl 2>/dev/null || true
nmcli dev set "$IF" managed no 2>/dev/null || true
nmcli dev disconnect "$IF" 2>/dev/null || true
sleep 1

# unload stock (or previous) stack in reverse-dependency order
modprobe -r rtw88_8822bu rtw88_usb rtw88_8822b rtw88_core 2>/dev/null || true
sleep 1

# load patched modules in dependency order
insmod ./rtw88_core.ko
insmod ./rtw88_usb.ko
insmod ./rtw88_8822b.ko
insmod ./rtw88_8822bu.ko

sleep 3
echo "loaded srcversion : $(cat /sys/module/rtw88_core/srcversion 2>/dev/null)"
echo "built  srcversion : $(modinfo ./rtw88_core.ko | awk '/^srcversion/{print $2}')"
echo "(they should match => patched driver is live)"
