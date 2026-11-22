#!/bin/bash
set -euo pipefail
dest=$1
install -D -m 755 "$CARGO_TARGET_DIR/aarch64-unknown-linux-musl/release/gh0st-term" "$dest/usr/bin/gh0st-term"
install -D -m 644 "$SRC/packaging/applications/terminal.desktop" \
    "$dest/usr/share/gh0st/applications/terminal.desktop"
