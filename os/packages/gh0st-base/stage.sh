#!/bin/bash
# Our apt source, its pin and the archive key, once the key exists
# (os/repo/README.md). Without it the image has no Gh0st OS source: a source
# apt can't check would fail every update check.
set -euo pipefail
dest=$1
repo=$SRC/os/repo
if [ -f "$repo/gh0st-archive-keyring.pgp" ]; then
    install -D -m 644 "$repo/gh0st-archive-keyring.pgp" \
        "$dest/usr/share/keyrings/gh0st-archive-keyring.pgp"
    install -D -m 644 "$repo/gh0st.sources" "$dest/etc/apt/sources.list.d/gh0st.sources"
    install -D -m 644 "$repo/gh0st.pref" "$dest/etc/apt/preferences.d/gh0st.pref"
else
    echo "gh0st-base: no os/repo/gh0st-archive-keyring.pgp, so no Gh0st OS source" >&2
fi
