#!/bin/bash
# Our apt source, its pin and the archive key, once the key exists
# (os/repo/README.md). Without it the image has no CPZeroOS source: a source
# apt can't check would fail every update check.
set -euo pipefail
dest=$1
repo=$SRC/os/repo
if [ -f "$repo/cpzeroos-archive-keyring.pgp" ]; then
    install -D -m 644 "$repo/cpzeroos-archive-keyring.pgp" \
        "$dest/usr/share/keyrings/cpzeroos-archive-keyring.pgp"
    install -D -m 644 "$repo/cpzeroos.sources" "$dest/etc/apt/sources.list.d/cpzeroos.sources"
    install -D -m 644 "$repo/cpzeroos.pref" "$dest/etc/apt/preferences.d/cpzeroos.pref"
else
    echo "cpzeroos-base: no os/repo/cpzeroos-archive-keyring.pgp, so no CPZeroOS source" >&2
fi
