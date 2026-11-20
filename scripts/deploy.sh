#!/usr/bin/env bash
# Builds cpz and installs it on a CardputerZero over SSH, next to the stock
# applaunch package. Usage:
#   CPZ_HOST=pi@cardputer.local scripts/deploy.sh            # install and restart
#   CPZ_HOST=pi@cardputer.local scripts/deploy.sh rollback   # back to stock
set -euo pipefail

host=${CPZ_HOST:?set CPZ_HOST, for example pi@cardputer.local}
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
    remote 'sudo rm -f /etc/systemd/user/APPLaunch.service.d/cpz.conf'
    restart_launcher
    echo "stock launcher restored on $host"
    exit 0
fi

cargo build --release --target "$target" -p cpz --manifest-path "$root/Cargo.toml"
bin="$root/target/$target/release/cpz"

scp -q "$bin" "$root/packaging/applications/store.desktop" \
    "$root/packaging/systemd/APPLaunch.service.d/cpz.conf" "$host:/tmp/"
remote 'set -e
    sudo install -m 755 /tmp/cpz /usr/bin/cpz
    sudo ln -sf cpz /usr/bin/cpz-shell
    sudo install -d /usr/share/cpz/applications /etc/systemd/user/APPLaunch.service.d
    sudo install -m 644 /tmp/store.desktop /usr/share/cpz/applications/store.desktop
    sudo install -m 644 /tmp/cpz.conf /etc/systemd/user/APPLaunch.service.d/cpz.conf
    rm -f /tmp/cpz /tmp/store.desktop /tmp/cpz.conf'
restart_launcher
echo "cpz $(git -C "$root" rev-parse --short HEAD) running on $host"
