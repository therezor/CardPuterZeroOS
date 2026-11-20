#!/usr/bin/env bash
# Builds zer0 and installs it on a CardputerZero over SSH, next to the stock
# applaunch package. Usage:
#   ZER0_HOST=pi@cardputer.local scripts/deploy.sh            # install and restart
#   ZER0_HOST=pi@cardputer.local scripts/deploy.sh rollback   # back to stock
set -euo pipefail

host=${ZER0_HOST:?set ZER0_HOST, for example pi@cardputer.local}
root=$(cd "$(dirname "$0")/.." && pwd)
target=aarch64-unknown-linux-musl
user_unit='systemctl --user'

remote() {
    # The command is built here on purpose and runs on the device.
    # shellcheck disable=SC2029
    ssh "$host" "$@"
}

restart_launcher() {
    # The launcher is a user unit of UID 1000 with linger, as in stock.
    remote "export XDG_RUNTIME_DIR=/run/user/\$(id -u); $user_unit daemon-reload; $user_unit restart APPLaunch.service"
}

if [ "${1:-}" = rollback ]; then
    remote 'sudo rm -f /etc/systemd/user/APPLaunch.service.d/zer0.conf'
    restart_launcher
    echo "stock launcher restored on $host"
    exit 0
fi

cargo build --release --target "$target" -p zer0 --manifest-path "$root/Cargo.toml"
bin="$root/target/$target/release/zer0"

scp -q "$bin" "$root/packaging/applications/store.desktop" \
    "$root/packaging/systemd/APPLaunch.service.d/zer0.conf" "$host:/tmp/"
remote 'set -e
    sudo install -m 755 /tmp/zer0 /usr/bin/zer0
    sudo ln -sf zer0 /usr/bin/zer0-shell
    sudo install -d /usr/share/zer0/applications /etc/systemd/user/APPLaunch.service.d
    sudo install -m 644 /tmp/store.desktop /usr/share/zer0/applications/store.desktop
    sudo install -m 644 /tmp/zer0.conf /etc/systemd/user/APPLaunch.service.d/zer0.conf
    rm -f /tmp/zer0 /tmp/store.desktop /tmp/zer0.conf'
restart_launcher
echo "zer0 $(git -C "$root" rev-parse --short HEAD) running on $host"
