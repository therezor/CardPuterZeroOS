#!/usr/bin/env bash
# Builds gh0st and installs it on a CardputerZero over SSH, next to the stock
# applaunch package. Usage:
#   GH0ST_HOST=pi@cardputer.local scripts/deploy.sh            # install and restart
#   GH0ST_HOST=pi@cardputer.local scripts/deploy.sh rollback   # back to stock
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

cargo build --release --target "$target" -p gh0st -p gh0st-settings -p gh0st-term -p gh0st-update --manifest-path "$root/Cargo.toml"
out="$root/target/$target/release"

scp -q "$out/gh0st" "$out/gh0st-settings" "$out/gh0st-term" "$out/gh0st-update" \
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
remote 'set -e
    sudo install -m 755 /tmp/gh0st /usr/bin/gh0st
    sudo install -m 755 /tmp/gh0st-settings /usr/bin/gh0st-settings
    sudo install -m 755 /tmp/gh0st-term /usr/bin/gh0st-term
    sudo install -m 755 /tmp/gh0st-update /usr/bin/gh0st-update
    sudo ln -sf gh0st /usr/bin/gh0st-shell
    # pkexec runs the helper by this path; a hard link, so it is the same
    # root-owned binary and not a symlink pkexec would refuse.
    sudo install -d /usr/libexec
    sudo ln -f /usr/bin/gh0st /usr/libexec/gh0st-helper
    sudo install -d /usr/share/gh0st/applications /etc/systemd/user/APPLaunch.service.d
    sudo install -m 644 /tmp/store.desktop /tmp/settings.desktop /tmp/terminal.desktop \
        /tmp/update.desktop /usr/share/gh0st/applications/
    cd /tmp/gh0st-update-units
    sudo install -m 644 -t /usr/lib/systemd/system gh0st-update-check.service \
        gh0st-update-daily.service gh0st-update-daily.timer gh0st-update-install.service
    sudo install -D -m 644 gh0st-update.tmpfiles /usr/lib/tmpfiles.d/gh0st-update.conf
    sudo install -D -m 755 90-gh0st-update /usr/lib/NetworkManager/dispatcher.d/90-gh0st-update
    cd /
    sudo systemd-tmpfiles --create gh0st-update.conf
    sudo install -m 644 /tmp/gh0st.conf /etc/systemd/user/APPLaunch.service.d/gh0st.conf
    sudo install -D -m 644 /tmp/fb_load-gh0st.conf /etc/systemd/system/fb_load.service.d/gh0st.conf
    sudo install -D -m 644 /tmp/50-gh0st.rules /etc/polkit-1/rules.d/50-gh0st.rules
    sudo install -D -m 644 /tmp/org.gh0st.policy /usr/share/polkit-1/actions/org.gh0st.policy
    # Its name before the rename: two policies on one exec path confuse pkexec.
    sudo rm -f /usr/share/polkit-1/actions/org.cpzero.gh0st.policy
    sudo systemctl daemon-reload
    rm -rf /tmp/gh0st /tmp/gh0st-settings /tmp/gh0st-term /tmp/gh0st-update /tmp/store.desktop \
        /tmp/settings.desktop /tmp/terminal.desktop /tmp/update.desktop /tmp/gh0st-update-units \
        /tmp/gh0st.conf /tmp/fb_load-gh0st.conf /tmp/50-gh0st.rules /tmp/org.gh0st.policy'
restart_launcher
echo "gh0st $(git -C "$root" rev-parse --short HEAD) running on $host"
