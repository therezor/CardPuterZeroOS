#!/bin/bash
# Installs the release binaries from the workspace build (step debs) and the
# files in packaging/, which scripts/deploy.sh installs the same way.
set -euo pipefail
dest=$1
out=$CARGO_TARGET_DIR/aarch64-unknown-linux-musl/release
pk=$SRC/packaging
install -D -m 755 "$out/gh0st" "$dest/usr/bin/gh0st"
install -m 755 "$out/gh0st-settings" "$dest/usr/bin/gh0st-settings"
install -m 755 "$out/gh0st-update" "$dest/usr/bin/gh0st-update"
install -m 755 "$out/gh0st-store" "$dest/usr/bin/gh0st-store"
install -m 755 "$out/gh0st-keyring" "$dest/usr/bin/gh0st-keyring"
install -m 755 "$out/gh0st-files" "$dest/usr/bin/gh0st-files"
install -m 755 "$out/gh0st-text" "$dest/usr/bin/gh0st-text"
# Tools that call xdg-open get gh0st open (preinst diverts xdg-utils').
ln -s gh0st "$dest/usr/bin/xdg-open"
install -m 755 "$out/gh0st-keyboard" "$dest/usr/bin/gh0st-keyboard"
ln -s gh0st "$dest/usr/bin/gh0st-shell"
# pkexec runs the helper by this path. A hard link: the same root-owned file,
# not a symlink, which pkexec refuses.
install -d "$dest/usr/libexec"
ln "$dest/usr/bin/gh0st" "$dest/usr/libexec/gh0st-helper"
install -D -m 644 -t "$dest/usr/share/gh0st/applications" \
    "$pk/applications/settings.desktop" "$pk/applications/store.desktop" \
    "$pk/applications/update.desktop" "$pk/applications/keyring.desktop" \
    "$pk/applications/files.desktop" "$pk/applications/text.desktop" \
    "$pk/applications/keyboard.desktop"
install -D -m 644 "$pk/polkit/50-gh0st.rules" "$dest/usr/share/polkit-1/rules.d/50-gh0st.rules"
install -D -m 644 "$pk/polkit/org.gh0st.policy" \
    "$dest/usr/share/polkit-1/actions/org.gh0st.policy"
install -d "$dest/usr/share/APPLaunch/applications"
# Example colour schemes, listed after the built-ins (docs/design.md).
install -D -m 644 -t "$dest/usr/share/gh0st/palettes" "$pk"/palettes/*.palette
# The update engine: its units, folders and network trigger (docs/updates.md).
up=$pk/systemd/update
install -D -m 644 -t "$dest/usr/lib/systemd/system" \
    "$up/gh0st-update-check.service" "$up/gh0st-update-daily.service" \
    "$up/gh0st-update-daily.timer" "$up/gh0st-update-install.service"
install -D -m 644 "$up/gh0st-update.tmpfiles" "$dest/usr/lib/tmpfiles.d/gh0st-update.conf"
install -D -m 755 "$up/90-gh0st-update" "$dest/usr/lib/NetworkManager/dispatcher.d/90-gh0st-update"
# The Store's engine: its unit and folders (docs/store.md).
st=$pk/systemd/store
install -m 644 -t "$dest/usr/lib/systemd/system" "$st/gh0st-store.service"
install -D -m 644 "$st/gh0st-store.tmpfiles" "$dest/usr/lib/tmpfiles.d/gh0st-store.conf"
# The keyring's vault folder (docs/keyring.md).
install -D -m 644 "$pk/systemd/keyring/gh0st-keyring.tmpfiles" \
    "$dest/usr/lib/tmpfiles.d/gh0st-keyring.conf"
