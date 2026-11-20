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
        /etc/systemd/system/fb_load.service.d/gh0st.conf && sudo systemctl daemon-reload'
    restart_launcher
    echo "stock launcher restored on $host"
    exit 0
fi

cargo build --release --target "$target" -p gh0st --manifest-path "$root/Cargo.toml"
bin="$root/target/$target/release/gh0st"

scp -q "$bin" "$root/packaging/applications/store.desktop" \
    "$root/packaging/systemd/APPLaunch.service.d/gh0st.conf" "$host:/tmp/"
scp -q "$root/packaging/systemd/fb_load.service.d/gh0st.conf" "$host:/tmp/fb_load-gh0st.conf"
remote 'set -e
    sudo install -m 755 /tmp/gh0st /usr/bin/gh0st
    sudo ln -sf gh0st /usr/bin/gh0st-shell
    sudo install -d /usr/share/gh0st/applications /etc/systemd/user/APPLaunch.service.d
    sudo install -m 644 /tmp/store.desktop /usr/share/gh0st/applications/store.desktop
    sudo install -m 644 /tmp/gh0st.conf /etc/systemd/user/APPLaunch.service.d/gh0st.conf
    sudo install -D -m 644 /tmp/fb_load-gh0st.conf /etc/systemd/system/fb_load.service.d/gh0st.conf
    sudo systemctl daemon-reload
    rm -f /tmp/gh0st /tmp/store.desktop /tmp/gh0st.conf /tmp/fb_load-gh0st.conf'
restart_launcher
echo "gh0st $(git -C "$root" rev-parse --short HEAD) running on $host"
