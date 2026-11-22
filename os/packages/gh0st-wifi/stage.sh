#!/bin/bash
# Firmware for USB Wi-Fi adapters whose drivers do monitor mode, from the
# firmware packages pinned in lock.json. It goes under firmware/updates,
# which the kernel searches first, so a full firmware-* package still
# installs beside it without a file conflict.
set -euo pipefail
dest=$1
fw=$dest/usr/lib/firmware/updates
src=$(mktemp -d)
trap 'rm -rf "$src"' EXIT

# take PACKAGE FILE...: FILE as it sits under /usr/lib/firmware.
take() {
    local pkg=$1 url f
    shift
    url=$(jq -r --arg p "$pkg" '.repack[] | select(.name == $p) | .url' "$LOCK")
    rm -rf "$src/x"
    dpkg-deb -x "$CACHE/debs/$(basename "$url")" "$src/x"
    for f in "$@"; do
        install -D -m 644 "$src/x/usr/lib/firmware/$f" "$fw/$f"
    done
}

take firmware-atheros htc_9271.fw htc_7010.fw
take firmware-mediatek rt2870.bin mediatek/mt7601u.bin mediatek/mt7610u.bin \
    mediatek/mt7662u.bin mediatek/mt7662u_rom_patch.bin \
    mediatek/WIFI_MT7961_patch_mcu_1_2_hdr.bin mediatek/WIFI_RAM_CODE_MT7961_1.bin
take firmware-realtek rtw88/rtw8723d_fw.bin rtw88/rtw8812a_fw.bin rtw88/rtw8814a_fw.bin \
    rtw88/rtw8821a_fw.bin rtw88/rtw8821c_fw.bin rtw88/rtw8822b_fw.bin rtw88/rtw8822c_fw.bin
# The name mt7601u asked for before linux-firmware moved it.
ln -s mediatek/mt7601u.bin "$fw/mt7601u.bin"
# One source package (firmware-nonfree), one licence file for all three.
install -D -m 644 "$src/x/usr/share/doc/firmware-realtek/copyright" \
    "$dest/usr/share/doc/gh0st-wifi/copyright"
