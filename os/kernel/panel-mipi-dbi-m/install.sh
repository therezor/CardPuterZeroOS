#!/bin/sh
# Builds and installs gh0st's panel driver on the CardputerZero, keeping M5's
# module to roll back to. Run as root on the device. Takes effect on reboot.
#   ./install.sh            build and install
#   ./install.sh uninstall  restore M5's module and drop the TE overlay
set -eu

kver=$(uname -r)
extra=/lib/modules/$kver/extra
backup=/var/lib/gh0st/backup
overlays=/boot/firmware/overlays
config=/boot/firmware/config.txt
line=dtoverlay=gh0st-panel-te

if [ "${1:-}" = uninstall ]; then
    if [ -f "$backup/panel-mipi-dbi-m.ko" ]; then
        install -m 644 "$backup/panel-mipi-dbi-m.ko" "$extra/panel-mipi-dbi-m.ko"
    fi
    sed -i "/^$line\$/d" "$config"
    rm -f "$overlays/gh0st-panel-te.dtbo"
    depmod -a
    echo "M5 panel driver restored; reboot to load it"
    exit 0
fi

cd "$(dirname "$0")"
make
mkdir -p "$backup"
if [ ! -f "$backup/panel-mipi-dbi-m.ko" ]; then
    cp "$extra/panel-mipi-dbi-m.ko" "$backup/panel-mipi-dbi-m.ko"
fi
install -m 644 panel-mipi-dbi-m.ko "$extra/panel-mipi-dbi-m.ko"
dtc -@ -q -I dts -O dtb -o "$overlays/gh0st-panel-te.dtbo" gh0st-panel-te.dts
if ! grep -qx "$line" "$config"; then
    # After the overlay that creates the panel node.
    sed -i "/^dtoverlay=cardputerzero-v5-overlay\$/a $line" "$config"
fi
depmod -a
echo "gh0st panel driver installed; reboot to load it"
