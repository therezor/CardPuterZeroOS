#!/bin/bash
# The shell and its daemons end to end: the real shell under --sim with
# gh0st keyring and gh0st session on a private bus, keys from a script,
# real clients asking. Linux only (CI, or the image container):
#
#   dbus-run-session -- crates/gh0st/tests/keyring_sim.sh
#
# GH0ST names a build of gh0st.
set -euo pipefail
GH0ST=$(command -v "${GH0ST:-gh0st}")
work=$(mktemp -d)
shell=
fail() {
    echo "FAIL: $*" >&2
    echo "--- shell log" >&2
    tail -40 "$work/shell.log" >&2 || true
    exit 1
}
cleanup() {
    exec 3>&- 2>/dev/null || true
    [ -n "$shell" ] && kill "$shell" 2>/dev/null
    wait 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT
mkdir -p "$work"/{home/.config/cardputerzero,frames,ours,stock,icons}
echo '{"gh0st": {"lock": {"password": true}}}' >"$work/home/.config/cardputerzero/config.json"
cat >"$work/chkpwd" <<'SH'
#!/usr/bin/python3
import sys
sys.exit(0 if sys.stdin.buffer.read() == b"pw\0" else 7)
SH
cat >"$work/passwd" <<'SH'
#!/bin/sh
echo "me P 2026-11-23 0 99999 7 -1"
SH
chmod +x "$work/chkpwd" "$work/passwd"
mkfifo "$work/keys"

export HOME=$work/home GH0ST_SERVICES=1 GH0ST_SERVICES_EXE=$GH0ST \
    GH0ST_KEYRING_VAULT=$work/login.vault GH0ST_CHKPWD=$work/chkpwd \
    GH0ST_PASSWD=$work/passwd GH0ST_APPS_OURS=$work/ours \
    GH0ST_APPS_STOCK=$work/stock GH0ST_ICON_ROOT=$work/icons GH0ST_LOG=debug
"$GH0ST" shell --sim "$work/frames" <"$work/keys" >"$work/shell.log" 2>&1 &
shell=$!
exec 3>"$work/keys"
say() { printf '%s\n' "$@" >&3; }
waitfor() {
    for _ in $(seq 100); do grep -q "$1" "$work/shell.log" && return 0; sleep 0.1; done
    fail "never saw: $1"
}

echo "unlock: the lock screen's password opens the vault"
sleep 1
say "tab" "enter" "wait 100" "type pw" "enter"
waitfor "keyring Unlocked"
[ -f "$work/login.vault" ] || fail "no vault"

echo "another app's secret: the shell asks, 5 allows once"
printf 'tok' | secret-tool store --label='GitHub token' service github.com
timeout 30 python3 - >"$work/py1" <<'PY' &
import secretstorage
c = secretstorage.dbus_init()
item = next(secretstorage.get_default_collection(c).search_items({"service": "github.com"}))
print("locked" if item.unlock() else item.get_secret().decode())
PY
py=$!
waitfor "AskApprove"
# A new question takes no answer for 600 ms (prompt::ARM_MS).
sleep 1
say "5"
wait "$py" || fail "the first lookup didn't finish"
[ "$(cat "$work/py1")" = tok ] || fail "allowed once, got: $(cat "$work/py1")"

echo "a key too soon after the question answers nothing; Esc denies"
timeout 30 python3 - >"$work/py2" <<'PY' &
import secretstorage
c = secretstorage.dbus_init()
item = next(secretstorage.get_default_collection(c).search_items({"service": "github.com"}))
print("locked" if item.unlock() else item.get_secret().decode())
PY
py=$!
waitfor "AskApprove { id: 2"
say "5"
sleep 1
[ -s "$work/py2" ] && fail "answered before it armed"
say "esc"
wait "$py" || fail "the second lookup didn't finish"
[ "$(cat "$work/py2")" = locked ] || fail "denied, got: $(cat "$work/py2")"

echo "an app in front asks: the shell parks it, asks, and brings it back"
cat >"$work/probe" <<SH
#!/bin/sh
timeout 30 python3 - >"$work/py3" <<'PY'
import secretstorage
c = secretstorage.dbus_init()
item = next(secretstorage.get_default_collection(c).search_items({"service": "github.com"}))
print("locked" if item.unlock() else item.get_secret().decode())
PY
sleep 30
SH
chmod +x "$work/probe"
printf '[Desktop Entry]\nType=Application\nName=Probe\nExec=%s\n' "$work/probe" >"$work/ours/probe.desktop"
kill -HUP "$shell"
sleep 1
say "enter"
waitfor "started Probe"
waitfor "AskApprove { id: 3"
sleep 1
say "5"
for _ in $(seq 100); do [ -s "$work/py3" ] && break; sleep 0.1; done
[ "$(cat "$work/py3")" = tok ] || fail "the app in front, got: $(cat "$work/py3")"
waitfor "resumed Probe"

echo "notify-send: a notice the shell picks up"
notify-send 'Build done' 'all green'
sleep 0.5
ls "$work/home/.local/state/gh0st/notify/" | grep -q notify-send || fail "no notice"

echo "all passed"
