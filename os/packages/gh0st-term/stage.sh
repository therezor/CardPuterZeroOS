#!/bin/bash
set -euo pipefail
dest=$1
install -D -m 755 "$CARGO_TARGET_DIR/aarch64-unknown-linux-musl/release/gh0st-term" "$dest/usr/bin/gh0st-term"
install -D -m 644 "$SRC/packaging/applications/terminal.desktop" \
    "$dest/usr/share/gh0st/applications/terminal.desktop"
# The same entry under freedesktop's folder, as gh0st-<name>.desktop: the
# id gio and xdg-mime write into mimeapps.list (docs/apps.md).
install -d "$dest/usr/share/applications"
for f in "$dest"/usr/share/gh0st/applications/*.desktop; do
    n=$(basename "$f")
    ln -s "../gh0st/applications/$n" "$dest/usr/share/applications/gh0st-$n"
done
