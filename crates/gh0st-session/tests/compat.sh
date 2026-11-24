#!/bin/sh
# Real clients against `gh0st session --test` on a private session bus:
# libnotify's notify-send and a ScreenSaver inhibit through gdbus.
#
#   dbus-run-session -- crates/gh0st-session/tests/compat.sh
set -eu
GH0ST=${GH0ST:-gh0st}
work=$(mktemp -d)
pid=
fail() { echo "FAIL: $*" >&2; exit 1; }
trap '[ -n "$pid" ] && kill $pid 2>/dev/null; rm -rf "$work"' EXIT
HOME=$work XDG_STATE_HOME=$work/.local/state "$GH0ST" session --test >"$work/out" &
pid=$!
i=0
until dbus-send --session --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus \
    org.freedesktop.DBus.NameHasOwner string:org.freedesktop.Notifications 2>/dev/null | grep -q true; do
    i=$((i + 1))
    [ $i -lt 100 ] || fail "the daemon never owned its name"
    sleep 0.05
done

echo "notify-send: a notice file"
notify-send -u critical 'Build done' 'all green'
f=$(ls "$work/.local/state/gh0st/notify/" | head -1)
[ -n "$f" ] || fail "no notice file"
grep -q 'title=Build done' "$work/.local/state/gh0st/notify/$f" || fail "title"
grep -q 'tone=critical' "$work/.local/state/gh0st/notify/$f" || fail "tone"

echo "gdbus: server information"
gdbus call --session --dest org.freedesktop.Notifications --object-path /org/freedesktop/Notifications \
    --method org.freedesktop.Notifications.GetServerInformation | grep -q gh0st || fail "server info"

echo "gdbus: inhibit and uninhibit"
cookie=$(gdbus call --session --dest org.freedesktop.ScreenSaver --object-path /org/freedesktop/ScreenSaver \
    --method org.freedesktop.ScreenSaver.Inhibit mpv playing | sed 's/[^0-9]//g')
[ -n "$cookie" ] || fail "no cookie"
gdbus call --session --dest org.freedesktop.ScreenSaver --object-path /ScreenSaver \
    --method org.freedesktop.ScreenSaver.UnInhibit "$cookie" >/dev/null || fail "uninhibit"
grep -q '^ready$' "$work/out" || fail "ready"

echo "all passed"
