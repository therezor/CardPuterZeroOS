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
mkdir -p "$work/files" "$work/.config/cardputerzero" && printf 'hi\n' >"$work/files/a note.txt"
HOME=$work XDG_STATE_HOME=$work/.local/state GH0ST_PORTAL_ANSWER="$work/files/a note.txt" \
    GH0ST_PORTAL_ACCOUNT=once "$GH0ST" session --test >"$work/out" &
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

echo "portal: Settings reads the look and signals a change"
gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.portal.Settings.ReadOne org.freedesktop.appearance color-scheme \
    | grep -q 'uint32 1' || fail "ReadOne color-scheme"
cat >"$work/look.py" <<'PY'
import sys
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
got = []
loop = GLib.MainLoop()
def changed(conn, sender, obj, iface, sig, params):
    got.append(params.unpack()[:2])
    loop.quit()
bus.signal_subscribe(None, "org.freedesktop.portal.Settings", "SettingChanged",
                     "/org/freedesktop/portal/desktop", None, Gio.DBusSignalFlags.NONE, changed)
r = bus.call_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
                  "org.freedesktop.portal.Settings", "ReadAll", GLib.Variant("(as)", ([""],)),
                  None, Gio.DBusCallFlags.NONE, -1, None)
assert "org.freedesktop.appearance" in r.unpack()[0], r.unpack()
with open(sys.argv[1] + ".tmp", "w") as f:
    f.write('{"gh0st": {"palette": "Paper"}}')
import os
os.rename(sys.argv[1] + ".tmp", sys.argv[1])
GLib.timeout_add(5000, loop.quit)
loop.run()
print(got)
PY
out=$(timeout 10 python3 "$work/look.py" "$work/.config/cardputerzero/config.json") || fail "Settings: $out"
echo "$out" | grep -q "org.freedesktop.appearance" || fail "SettingChanged: $out"

echo "portal: Inhibit keeps the caller awake until it leaves"
gdbus call --session --dest org.freedesktop.portal.Desktop --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.portal.Inhibit.Inhibit '' 8 '{}' | grep -q request || fail "Inhibit"
i=0
until grep -q '^awake pid:[0-9]* 0$' "$work/out"; do
    i=$((i + 1))
    [ $i -lt 40 ] || fail "Inhibit never ended"
    sleep 0.05
done
grep -q '^awake pid:[0-9]* 1$' "$work/out" || fail "Inhibit never began"

echo "portal: Account answers after the user's yes"
cat >"$work/account.py" <<'PY'
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
me = bus.get_unique_name()[1:].replace(".", "_")
path = "/org/freedesktop/portal/desktop/request/%s/a1" % me
got = {}
loop = GLib.MainLoop()
def response(conn, sender, obj, iface, sig, params):
    got["r"] = params.unpack()
    loop.quit()
bus.signal_subscribe(None, "org.freedesktop.portal.Request", "Response", path, None,
                     Gio.DBusSignalFlags.NONE, response)
bus.call_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
              "org.freedesktop.portal.Account", "GetUserInformation",
              GLib.Variant("(sa{sv})", ("", {"handle_token": GLib.Variant("s", "a1")})),
              None, Gio.DBusCallFlags.NONE, -1, None)
GLib.timeout_add(5000, loop.quit)
loop.run()
code, results = got["r"]
print(code, results.get("id"))
PY
out=$(timeout 10 python3 "$work/account.py") || fail "Account: $out"
echo "$out" | grep -q "^0 [^N]" || fail "Account answer: $out"

echo "gh0st open and xdg-open: a file through OpenURI"
timeout 10 "$GH0ST" open "$work/files/a note.txt" || fail "gh0st open"
ln -sf "$GH0ST" "$work/xdg-open"
timeout 10 "$work/xdg-open" "$work/files" || fail "xdg-open"
"$GH0ST" open "$work/nope" 2>/dev/null && fail "gh0st open took a missing file"

echo "notifications: actions come back to the poster"
kill $pid 2>/dev/null; wait $pid 2>/dev/null || true
HOME=$work XDG_STATE_HOME=$work/.local/state GH0ST_NOTIFY_PRESS=1 \
    "$GH0ST" session --test >"$work/out3" &
pid=$!
for _ in $(seq 100); do
    gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
        --method org.freedesktop.DBus.NameHasOwner org.freedesktop.Notifications 2>/dev/null \
        | grep -q true && break
    sleep 0.05
done
gdbus call --session --dest org.freedesktop.Notifications --object-path /org/freedesktop/Notifications \
    --method org.freedesktop.Notifications.GetCapabilities | grep -q actions || fail "actions capability"
got=$(timeout 10 notify-send -A reply=Reply -A spam=Spam 'New mail' 'From Ana') || fail "notify-send -A: $got"
[ "$got" = reply ] || fail "notify-send heard: $got"
cat >"$work/note.py" <<'PY'
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SESSION)
got = {}
loop = GLib.MainLoop()
def invoked(conn, sender, obj, iface, sig, params):
    got["a"] = params.unpack()
    loop.quit()
bus.signal_subscribe(None, "org.freedesktop.portal.Notification", "ActionInvoked",
                     "/org/freedesktop/portal/desktop", None, Gio.DBusSignalFlags.NONE, invoked)
button = {"label": GLib.Variant("s", "Next"), "action": GLib.Variant("s", "app.next"),
          "target": GLib.Variant("u", 7)}
n = {"title": GLib.Variant("s", "Night Drive"), "buttons": GLib.Variant("aa{sv}", [button])}
bus.call_sync("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
              "org.freedesktop.portal.Notification", "AddNotification",
              GLib.Variant("(sa{sv})", ("song", n)), None, Gio.DBusCallFlags.NONE, -1, None)
GLib.timeout_add(5000, loop.quit)
loop.run()
print(got.get("a"))
PY
out=$(timeout 10 python3 "$work/note.py") || fail "portal notification: $out"
echo "$out" | grep -q "('song', 'app.next', \[7\])" || fail "portal ActionInvoked: $out"

if [ -n "${GH0ST_FILES:-}" ]; then
    echo "portal: the real chooser picks a file with keys"
    kill $pid 2>/dev/null; wait $pid 2>/dev/null || true
    printf 'wait 300\nenter\nwait 300\n' >"$work/keys"
    mkdir -p "$work/home"
    printf 'x\n' >"$work/home/pick me.txt"
    HOME=$work/home XDG_STATE_HOME=$work/.local/state GH0ST_PORTAL_CHOOSER=$GH0ST_FILES \
        GH0ST_PORTAL_KEYS=$work/keys "$GH0ST" session --test >"$work/out2" &
    pid=$!
    for _ in $(seq 100); do
        gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
            --method org.freedesktop.DBus.NameHasOwner org.freedesktop.portal.Desktop 2>/dev/null \
            | grep -q true && break
        sleep 0.05
    done
    out=$(timeout 20 python3 "$work/pick.py" OpenFile) || fail "chooser: $out"
    echo "$out" | grep -q "^0 \['file://.*/pick%20me.txt'\]" || fail "chooser answer: $out"
fi

echo "all passed"
