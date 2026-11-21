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
    # The launcher is a user unit of UID 1000 with linger, as in stock.
    remote "export XDG_RUNTIME_DIR=/run/user/\$(id -u); $user_unit daemon-reload; $user_unit restart APPLaunch.service"
}

if [ "${1:-}" = rollback ]; then
    remote 'sudo rm -f /etc/systemd/user/APPLaunch.service.d/gh0st.conf \
        /etc/systemd/system/fb_load.service.d/gh0st.conf \
        /etc/polkit-1/rules.d/50-gh0st.rules \
        /usr/share/polkit-1/actions/org.cpzero.gh0st.policy \
        /usr/libexec/gh0st-helper /etc/udev/rules.d/61-gh0st-cpu.rules \
        && sudo systemctl daemon-reload'
    restart_launcher
    echo "stock launcher restored on $host"
    exit 0
fi

cargo build --release --target "$target" -p gh0st -p gh0st-settings -p gh0st-term --manifest-path "$root/Cargo.toml"
out="$root/target/$target/release"

scp -q "$out/gh0st" "$out/gh0st-settings" "$out/gh0st-term" \
    "$root/packaging/applications/store.desktop" \
    "$root/packaging/applications/settings.desktop" \
    "$root/packaging/applications/terminal.desktop" \
    "$root/packaging/systemd/APPLaunch.service.d/gh0st.conf" "$host:/tmp/"
scp -q "$root/packaging/systemd/fb_load.service.d/gh0st.conf" "$host:/tmp/fb_load-gh0st.conf"
scp -q "$root/packaging/polkit/50-gh0st.rules" "$root/packaging/polkit/org.cpzero.gh0st.policy" "$host:/tmp/"
remote 'set -e
    sudo install -m 755 /tmp/gh0st /usr/bin/gh0st
    sudo install -m 755 /tmp/gh0st-settings /usr/bin/gh0st-settings
    sudo install -m 755 /tmp/gh0st-term /usr/bin/gh0st-term
    sudo ln -sf gh0st /usr/bin/gh0st-shell
    # pkexec runs the helper by this path; a hard link, so it is the same
    # root-owned binary and not a symlink pkexec would refuse.
    sudo install -d /usr/libexec
    sudo ln -f /usr/bin/gh0st /usr/libexec/gh0st-helper
    sudo install -d /usr/share/gh0st/applications /etc/systemd/user/APPLaunch.service.d
    sudo install -m 644 /tmp/store.desktop /tmp/settings.desktop /tmp/terminal.desktop \
        /usr/share/gh0st/applications/
    sudo install -m 644 /tmp/gh0st.conf /etc/systemd/user/APPLaunch.service.d/gh0st.conf
    sudo install -D -m 644 /tmp/fb_load-gh0st.conf /etc/systemd/system/fb_load.service.d/gh0st.conf
    sudo install -D -m 644 /tmp/50-gh0st.rules /etc/polkit-1/rules.d/50-gh0st.rules
    sudo install -D -m 644 /tmp/org.cpzero.gh0st.policy /usr/share/polkit-1/actions/org.cpzero.gh0st.policy
    sudo systemctl daemon-reload
    rm -f /tmp/gh0st /tmp/gh0st-settings /tmp/gh0st-term /tmp/store.desktop /tmp/settings.desktop \
        /tmp/terminal.desktop \
        /tmp/gh0st.conf /tmp/fb_load-gh0st.conf /tmp/50-gh0st.rules /tmp/org.cpzero.gh0st.policy'
restart_launcher
echo "gh0st $(git -C "$root" rev-parse --short HEAD) running on $host"
