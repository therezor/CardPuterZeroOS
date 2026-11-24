#!/bin/bash
set -euo pipefail
dest=$1
install -D -m 755 "$CARGO_TARGET_DIR/aarch64-unknown-linux-musl/release/gh0st-media" "$dest/usr/bin/gh0st-media"
install -D -m 755 "$WORK/mediad/gh0st-mediad" "$dest/usr/libexec/gh0st-mediad"
install -D -m 644 "$SRC/packaging/applications/media.desktop" \
    "$dest/usr/share/gh0st/applications/media.desktop"
