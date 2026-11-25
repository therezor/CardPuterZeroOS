#!/usr/bin/env bash
# Builds gh0st and installs it on a CardputerZero over SSH, next to the stock
# applaunch package. Usage:
#   GH0ST_HOST=pi@cardputer.local scripts/deploy.sh            # install and restart
#   GH0ST_HOST=pi@cardputer.local scripts/deploy.sh rollback   # back to stock
#   GH0ST_HOST=pi@cardputer.local scripts/deploy.sh camera     # the camera app alone
#   GH0ST_HOST=pi@cardputer.local scripts/deploy.sh media      # the media app alone
# The camera's helper is C++ against the device's libcamera, so it builds in
# the image container (os/build.sh camd): Docker must be running.
set -euo pipefail

host=${GH0ST_HOST:?set GH0ST_HOST, for example pi@cardputer.local}
root=$(cd "$(dirname "$0")/.." && pwd)
target=aarch64-unknown-linux-musl
user_unit='systemctl --user'

remote() {
    # The command is built here on purpose and runs on the device. -t gives
    # sudo a terminal, so it can ask for the password.
    # shellcheck disable=SC2029
    ssh -t "$host" "$@"
}

restart_launcher() {
    # A user unit of UID 1000 with linger: gh0st.service on our image,
    # APPLaunch.service with our drop-in on the stock one.
    remote "export XDG_RUNTIME_DIR=/run/user/\$(id -u); $user_unit daemon-reload
        $user_unit reload dbus.service 2>/dev/null || true
        $user_unit try-restart pipewire-pulse.socket wireplumber.service
        if $user_unit cat gh0st.service >/dev/null 2>&1; then
            $user_unit restart gh0st.service
        else
            $user_unit restart APPLaunch.service
        fi"
}

if [ "${1:-}" = rollback ]; then
    remote 'sudo rm -f /etc/systemd/user/APPLaunch.service.d/gh0st.conf \
        /etc/systemd/system/fb_load.service.d/gh0st.conf \
        /etc/polkit-1/rules.d/50-gh0st.rules \
        /usr/share/polkit-1/actions/org.gh0st.policy \
        /usr/share/polkit-1/actions/org.cpzero.gh0st.policy \
        /usr/libexec/gh0st-helper /etc/udev/rules.d/61-gh0st-cpu.rules \
        /etc/rpi/swap.conf.d/90-gh0st-swap.conf /etc/sysctl.d/90-gh0st-swap.conf \
        /etc/systemd/zram-generator.conf.d/90-gh0st-swap-size.conf \
        /etc/systemd/zram-generator.conf.d/90-gh0st-swap-compression.conf \
        /etc/NetworkManager/conf.d/90-gh0st-mac.conf \
        /etc/NetworkManager/conf.d/90-gh0st-scan-mac.conf \
        /etc/NetworkManager/conf.d/90-gh0st-dhcp-hostname.conf \
        /etc/NetworkManager/conf.d/90-gh0st-ip6-privacy.conf \
        /etc/NetworkManager/conf.d/90-gh0st-wifi-powersave.conf \
        /etc/NetworkManager/system-connections/gh0st-hotspot.nmconnection \
        /etc/udev/rules.d/61-gh0st-usb-power.rules \
        && sudo systemctl daemon-reload && sudo nmcli general reload conf \
        && sudo nmcli connection reload'
    restart_launcher
    echo "stock launcher restored on $host"
    exit 0
fi

if [ "${1:-}" = camera ]; then
    "$root/os/build.sh" camd
    cargo build --release --target "$target" -p gh0st-camera --manifest-path "$root/Cargo.toml"
    scp -q "$root/target/$target/release/gh0st-camera" "$root/os/out/camd/gh0st-camd" \
        "$root/packaging/applications/camera.desktop" "$root/crates/gh0st-camera/camd/vc4.yaml" \
        "$root/crates/gh0st-camera/camd/vc4-photo.yaml" "$host:/tmp/"
    remote 'set -e
        sudo install -m 755 /tmp/gh0st-camera /usr/bin/gh0st-camera
        sudo install -D -m 755 /tmp/gh0st-camd /usr/libexec/gh0st-camd
        sudo install -D -m 644 /tmp/camera.desktop /usr/share/gh0st/applications/camera.desktop
        sudo install -D -m 644 /tmp/vc4.yaml /usr/share/gh0st-camera/vc4.yaml
        sudo install -D -m 644 /tmp/vc4-photo.yaml /usr/share/gh0st-camera/vc4-photo.yaml
        rm -f /tmp/gh0st-camera /tmp/gh0st-camd /tmp/camera.desktop /tmp/vc4.yaml /tmp/vc4-photo.yaml'
    restart_launcher
    echo "camera installed on $host; measure with: ssh $host /usr/libexec/gh0st-camd --probe ~"
    exit 0
fi

if [ "${1:-}" = media ]; then
    "$root/os/build.sh" mediad
    cargo build --release --target "$target" -p gh0st-media --manifest-path "$root/Cargo.toml"
    scp -q "$root/target/$target/release/gh0st-media" "$root/os/out/mediad/gh0st-mediad" \
        "$root/packaging/applications/media.desktop" "$host:/tmp/"
    remote 'set -e
        sudo apt-get install -y --no-install-recommends libheif1 libheif-plugin-ffmpegdec libraw23 \
            libwebpdemux2 libass9 libavfilter10
        sudo install -m 755 /tmp/gh0st-media /usr/bin/gh0st-media
        sudo install -D -m 755 /tmp/gh0st-mediad /usr/libexec/gh0st-mediad
        sudo install -D -m 644 /tmp/media.desktop /usr/share/gh0st/applications/media.desktop
        rm -f /tmp/gh0st-media /tmp/gh0st-mediad /tmp/media.desktop'
    echo "media installed on $host"
    exit 0
fi

cargo build --release --target "$target" -p gh0st -p gh0st-settings -p gh0st-term -p gh0st-update \
    -p gh0st-store -p gh0st-keyring -p gh0st-files -p gh0st-text -p gh0st-keyboard \
    --manifest-path "$root/Cargo.toml"
out="$root/target/$target/release"

scp -q "$out/gh0st" "$out/gh0st-settings" "$out/gh0st-term" "$out/gh0st-update" "$out/gh0st-store" \
    "$out/gh0st-keyring" "$out/gh0st-files" "$out/gh0st-text" "$out/gh0st-keyboard" \
    "$root/packaging/applications/keyring.desktop" \
    "$root/packaging/applications/keyboard.desktop" \
    "$root/packaging/applications/files.desktop" "$root/packaging/applications/text.desktop" \
    "$root/packaging/applications/store.desktop" \
    "$root/packaging/applications/settings.desktop" \
    "$root/packaging/applications/terminal.desktop" \
    "$root/packaging/applications/update.desktop" \
    "$root/packaging/systemd/APPLaunch.service.d/gh0st.conf" "$host:/tmp/"
scp -q "$root/packaging/systemd/fb_load.service.d/gh0st.conf" "$host:/tmp/fb_load-gh0st.conf"
scp -q "$root/packaging/polkit/50-gh0st.rules" "$root/packaging/polkit/org.gh0st.policy" "$host:/tmp/"
# The update engine's units, folders and network trigger (docs/updates.md).
# Installed, not enabled: the daily check is Settings' to turn on.
scp -q -r "$root/packaging/systemd/update" "$host:/tmp/gh0st-update-units"
scp -q -r "$root/packaging/systemd/store" "$host:/tmp/gh0st-store-units"
scp -q "$root/packaging/systemd/keyring/gh0st-keyring.tmpfiles" "$host:/tmp/"
scp -q -r "$root/packaging/palettes" "$host:/tmp/gh0st-palettes"
# A paused app's null sink must never take a new stream (gh0st audio).
scp -q "$root/os/packages/gh0st-base/root/usr/share/wireplumber/wireplumber.conf.d/50-gh0st-streams.conf" \
    "$host:/tmp/"
# The sandbox's bus policy and audio rules (docs/isolation.md). Without
# bwrap the shell starts no sandboxed app at all.
base="$root/os/packages/gh0st-base/root/usr"
scp -q "$base/share/dbus-1/session.d/gh0st.conf" "$host:/tmp/gh0st-session.conf"
scp -q "$base/share/wireplumber/scripts/gh0st-nomic.lua" \
    "$base/share/wireplumber/wireplumber.conf.d/50-gh0st-nomic.conf" "$host:/tmp/"
scp -q "$base/share/pipewire/pipewire-pulse.conf.d/50-gh0st-nomic.conf" "$host:/tmp/gh0st-nomic-pulse.conf"
scp -q "$base/lib/systemd/user/pipewire-pulse.socket.d/gh0st.conf" "$host:/tmp/gh0st-pulse-socket.conf"
scp -q "$root/os/packages/gh0st/root/usr/lib/systemd/user/gh0st.service" "$host:/tmp/"
remote 'set -e
    sudo install -m 755 /tmp/gh0st /usr/bin/gh0st
    sudo install -m 755 /tmp/gh0st-settings /usr/bin/gh0st-settings
    sudo install -m 755 /tmp/gh0st-term /usr/bin/gh0st-term
    sudo install -m 755 /tmp/gh0st-update /usr/bin/gh0st-update
    sudo install -m 755 /tmp/gh0st-store /usr/bin/gh0st-store
    sudo install -m 755 /tmp/gh0st-keyring /usr/bin/gh0st-keyring
    sudo install -m 755 /tmp/gh0st-files /usr/bin/gh0st-files
    sudo install -m 755 /tmp/gh0st-text /usr/bin/gh0st-text
    sudo ln -sf gh0st /usr/bin/xdg-open
    sudo install -m 755 /tmp/gh0st-keyboard /usr/bin/gh0st-keyboard
    sudo ln -sf gh0st /usr/bin/gh0st-shell
    # The keyring vault folder, outside the home folder.
    sudo install -m 644 /tmp/gh0st-keyring.tmpfiles /usr/lib/tmpfiles.d/gh0st-keyring.conf
    sudo systemd-tmpfiles --create /usr/lib/tmpfiles.d/gh0st-keyring.conf
    # pkexec runs the helper by this path; a hard link, so it is the same
    # root-owned binary and not a symlink pkexec would refuse.
    sudo install -d /usr/libexec
    sudo ln -f /usr/bin/gh0st /usr/libexec/gh0st-helper
    sudo install -d /usr/share/gh0st/applications /etc/systemd/user/APPLaunch.service.d
    sudo install -m 644 /tmp/store.desktop /tmp/settings.desktop /tmp/terminal.desktop \
        /tmp/update.desktop /tmp/keyring.desktop /tmp/files.desktop /tmp/text.desktop \
        /tmp/keyboard.desktop \
        /usr/share/gh0st/applications/
    cd /tmp/gh0st-update-units
    sudo install -m 644 -t /usr/lib/systemd/system gh0st-update-check.service \
        gh0st-update-daily.service gh0st-update-daily.timer gh0st-update-install.service
    sudo install -D -m 644 gh0st-update.tmpfiles /usr/lib/tmpfiles.d/gh0st-update.conf
    sudo install -D -m 755 90-gh0st-update /usr/lib/NetworkManager/dispatcher.d/90-gh0st-update
    sudo install -D -m 644 -t /usr/share/gh0st/palettes /tmp/gh0st-palettes/*.palette
    sudo install -D -m 644 -t /usr/share/wireplumber/wireplumber.conf.d /tmp/50-gh0st-streams.conf
    sudo apt-get install -y --no-install-recommends bubblewrap
    sudo install -D -m 644 /tmp/gh0st-session.conf /usr/share/dbus-1/session.d/gh0st.conf
    sudo install -D -m 644 /tmp/gh0st-nomic.lua /usr/share/wireplumber/scripts/gh0st-nomic.lua
    sudo install -D -m 644 -t /usr/share/wireplumber/wireplumber.conf.d /tmp/50-gh0st-nomic.conf
    sudo install -D -m 644 /tmp/gh0st-nomic-pulse.conf \
        /usr/share/pipewire/pipewire-pulse.conf.d/50-gh0st-nomic.conf
    sudo install -D -m 644 /tmp/gh0st-pulse-socket.conf \
        /usr/lib/systemd/user/pipewire-pulse.socket.d/gh0st.conf
    # Our image runs the shell as gh0st.service; it now delegates cgroups.
    if [ -f /usr/lib/systemd/user/gh0st.service ]; then
        sudo install -m 644 /tmp/gh0st.service /usr/lib/systemd/user/gh0st.service
    fi
    cd /tmp/gh0st-store-units
    sudo install -m 644 -t /usr/lib/systemd/system gh0st-store.service
    sudo install -D -m 644 gh0st-store.tmpfiles /usr/lib/tmpfiles.d/gh0st-store.conf
    cd /
    sudo systemd-tmpfiles --create gh0st-update.conf gh0st-store.conf
    sudo install -m 644 /tmp/gh0st.conf /etc/systemd/user/APPLaunch.service.d/gh0st.conf
    sudo install -D -m 644 /tmp/fb_load-gh0st.conf /etc/systemd/system/fb_load.service.d/gh0st.conf
    sudo install -D -m 644 /tmp/50-gh0st.rules /etc/polkit-1/rules.d/50-gh0st.rules
    sudo install -D -m 644 /tmp/org.gh0st.policy /usr/share/polkit-1/actions/org.gh0st.policy
    # Its name before the rename: two policies on one exec path confuse pkexec.
    sudo rm -f /usr/share/polkit-1/actions/org.cpzero.gh0st.policy
    sudo systemctl daemon-reload
    rm -rf /tmp/gh0st /tmp/gh0st-settings /tmp/gh0st-term /tmp/gh0st-update /tmp/gh0st-store \
        /tmp/gh0st-keyring /tmp/gh0st-files /tmp/keyring.desktop /tmp/files.desktop \
        /tmp/gh0st-store-units /tmp/gh0st-palettes /tmp/store.desktop \
        /tmp/settings.desktop /tmp/terminal.desktop /tmp/update.desktop /tmp/gh0st-update-units \
        /tmp/gh0st.conf /tmp/fb_load-gh0st.conf /tmp/50-gh0st.rules /tmp/org.gh0st.policy \
        /tmp/50-gh0st-streams.conf /tmp/gh0st-session.conf /tmp/gh0st-nomic.lua \
        /tmp/50-gh0st-nomic.conf /tmp/gh0st-nomic-pulse.conf /tmp/gh0st-pulse-socket.conf \
        /tmp/gh0st.service'
restart_launcher
echo "gh0st $(git -C "$root" rev-parse --short HEAD) running on $host"
