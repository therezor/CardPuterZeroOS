#!/bin/bash
set -euo pipefail
dest=$1
url=$(jq -r '.repack[] | select(.name == "applaunch") | .url' "$LOCK")
src=$(mktemp -d)
dpkg-deb -x "$CACHE/debs/$(basename "$url")" "$src"
s=$src/usr/share/APPLaunch
d=$dest/usr/share/APPLaunch
install -D -m 755 "$s/bin/M5CardputerZero-AppStore" "$d/bin/M5CardputerZero-AppStore"
install -d "$d/share/images" "$d/share/font" "$d/share/audio"
cp -a "$s"/share/images/appstore*.png "$s"/share/images/store_*.png "$d/share/images/"
# The fonts it names, minus the 45 MB Noto CJK pair. Spike S7 checks that
# it still renders with these.
cp -a "$s/share/font/AlibabaPuHuiTi-3-55-Regular.ttf" "$s/share/font/DejaVuSans.ttf" \
    "$s/share/font/DejaVuSerif.ttf" "$d/share/font/"
cp -a "$s/share/audio/Ding2.wav" "$s/share/audio/key_back.wav" "$d/share/audio/"
rm -rf "$src"
