#!/bin/bash
# Build the patched rtw88 driver (core + usb + 8822b + 8822bu) out-of-tree
# against the running Ubuntu kernel. Source is mainline v6.8 rtw88 with the
# monitor-injection TX-report fix applied to tx.c.
set -e
cd "$(dirname "$0")"
KDIR=/lib/modules/$(uname -r)/build
make -C "$KDIR" M="$PWD" modules \
  CONFIG_RTW88_CORE=m CONFIG_RTW88_USB=m \
  CONFIG_RTW88_8822B=m CONFIG_RTW88_8822BU=m \
  CONFIG_PM=y CONFIG_RTW88_DEBUG=y CONFIG_RTW88_DEBUGFS=y \
  KBUILD_MODPOST_WARN=1
echo "built: rtw88_core.ko rtw88_usb.ko rtw88_8822b.ko rtw88_8822bu.ko"
