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
mkdir -p "$work/files" && printf 'hi\n' >"$work/files/a note.txt"
HOME=$work XDG_STATE_HOME=$work/.local/state GH0ST_PORTAL_ANSWER="$work/files/a note.txt" \
    "$GH0ST" session --test >"$work/out" &
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

echo "portal: versions"
gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.DBus.Properties.Get org.freedesktop.portal.FileChooser version | grep -q 'uint32 4' \
    || fail "FileChooser version"
gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.DBus.Properties.Get org.freedesktop.portal.OpenURI version | grep -q 'uint32 5' \
    || fail "OpenURI version"
gdbus introspect --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop \
    | grep -q 'SaveFiles' || fail "introspection"

echo "portal: OpenFile through a Gio client, as libportal asks"
cat >"$work/pick.py" <<'PY'
import sys
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
me = bus.get_unique_name()[1:].replace(".", "_")
path = "/org/freedesktop/portal/desktop/request/%s/t1" % me
got = {}
loop = GLib.MainLoop()
def response(conn, sender, obj, iface, sig, params):
    got["r"] = params.unpack()
    loop.quit()
bus.signal_subscribe(None, "org.freedesktop.portal.Request", "Response", path, None,
                     Gio.DBusSignalFlags.NONE, response)
opts = {"handle_token": GLib.Variant("s", "t1"),
        "filters": GLib.Variant("a(sa(us))", [("Text", [(1, "text/plain")])])}
r = bus.call_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
                  "org.freedesktop.portal.FileChooser", sys.argv[1],
                  GLib.Variant("(ssa{sv})", ("", "Pick one", opts)), None,
                  Gio.DBusCallFlags.NONE, -1, None)
assert r.unpack()[0] == path, r.unpack()
GLib.timeout_add(5000, loop.quit)
loop.run()
code, results = got["r"]
print(code, results.get("uris"))
PY
out=$(timeout 10 python3 "$work/pick.py" OpenFile) || fail "OpenFile: $out"
echo "$out" | grep -q "^0 \['file://.*/a%20note.txt'\]" || fail "OpenFile answer: $out"
out=$(timeout 10 python3 "$work/pick.py" SaveFile) || fail "SaveFile: $out"
echo "$out" | grep -q "^0 " || fail "SaveFile answer: $out"

echo "portal: OpenURI takes file:// and nothing else"
gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.portal.OpenURI.OpenURI '' "file://$work/files" '{}' | grep -q request \
    || fail "OpenURI file"
gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.portal.OpenURI.OpenURI '' 'https://example.com' '{}' 2>/dev/null \
    && fail "OpenURI took a web address"
gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.portal.OpenURI.SchemeSupported file '{}' | grep -q true || fail "SchemeSupported"

echo "all passed"
