#!/bin/bash
# Installs the release binaries from the workspace build (step debs) and the
# files in packaging/, which scripts/deploy.sh installs the same way.
set -euo pipefail
dest=$1
out=$CARGO_TARGET_DIR/aarch64-unknown-linux-musl/release
pk=$SRC/packaging
install -D -m 755 "$out/gh0st" "$dest/usr/bin/gh0st"
install -m 755 "$out/gh0st-settings" "$dest/usr/bin/gh0st-settings"
ln -s gh0st "$dest/usr/bin/gh0st-shell"
# pkexec runs the helper by this path. A hard link: the same root-owned file,
# not a symlink, which pkexec refuses.
install -d "$dest/usr/libexec"
ln "$dest/usr/bin/gh0st" "$dest/usr/libexec/gh0st-helper"
install -D -m 644 -t "$dest/usr/share/gh0st/applications" \
    "$pk/applications/settings.desktop" "$pk/applications/store.desktop"
install -D -m 644 "$pk/polkit/50-gh0st.rules" "$dest/usr/share/polkit-1/rules.d/50-gh0st.rules"
install -D -m 644 "$pk/polkit/org.cpzero.gh0st.policy" \
    "$dest/usr/share/polkit-1/actions/org.cpzero.gh0st.policy"
install -d "$dest/usr/share/APPLaunch/applications"
