#!/bin/bash
set -euo pipefail
dest=$1
install -D -m 755 "$CARGO_TARGET_DIR/aarch64-unknown-linux-musl/release/gh0st-media" "$dest/usr/bin/gh0st-media"
install -D -m 755 "$WORK/mediad/gh0st-mediad" "$dest/usr/libexec/gh0st-mediad"
install -D -m 644 "$SRC/packaging/applications/media.desktop" \
    "$dest/usr/share/gh0st/applications/media.desktop"
# The same entry under freedesktop's folder, as gh0st-<name>.desktop: the
# id gio and xdg-mime write into mimeapps.list (docs/apps.md).
install -d "$dest/usr/share/applications"
for f in "$dest"/usr/share/gh0st/applications/*.desktop; do
    n=$(basename "$f")
    ln -s "../gh0st/applications/$n" "$dest/usr/share/applications/gh0st-$n"
done
