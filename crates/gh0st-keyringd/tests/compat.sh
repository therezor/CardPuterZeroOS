#!/bin/sh
# Real clients against `gh0st keyring --test` on a private session bus:
# libsecret (secret-tool, git-credential-libsecret), python keyring and
# secretstorage, gdbus. Our own client proves nothing about compatibility.
#
#   dbus-run-session -- crates/gh0st-keyringd/tests/compat.sh
#
# GH0ST names the gh0st binary; GIT_LIBSECRET the git helper.
set -eu
GH0ST=${GH0ST:-gh0st}
GIT_LIBSECRET=${GIT_LIBSECRET:-/usr/lib/git-core/git-credential-libsecret}
work=$(mktemp -d)
pid=
fail() { echo "FAIL: $*" >&2; exit 1; }
start() {
    [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null || true
    GH0ST_KEYRING_TEST_PASSWORD=pw GH0ST_KEYRING_TEST_ANSWER=$1 \
        "$GH0ST" keyring --test --vault "$work/login.vault" &
    pid=$!
    i=0
    until dbus-send --session --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus \
        org.freedesktop.DBus.NameHasOwner string:org.freedesktop.secrets 2>/dev/null | grep -q true; do
        i=$((i + 1))
        [ $i -lt 100 ] || fail "the daemon never owned its name"
        sleep 0.05
    done
}
trap '[ -n "$pid" ] && kill $pid 2>/dev/null; rm -rf "$work"' EXIT

start always

echo "secret-tool: store, lookup, search, clear"
printf 'hunter2' | secret-tool store --label='GitHub' service github.com user me
[ "$(secret-tool lookup service github.com user me)" = hunter2 ] || fail "lookup"
secret-tool search service github.com 2>&1 | grep -q 'label = GitHub' || fail "search"
printf 'second' | secret-tool store --label='GitHub' service github.com user me
[ "$(secret-tool lookup service github.com user me)" = second ] || fail "replace"
secret-tool clear service github.com user me
secret-tool lookup service github.com user me && fail "cleared item still there"

echo "gdbus: introspection and the default alias"
gdbus introspect --session --dest org.freedesktop.secrets --object-path /org/freedesktop/secrets \
    | grep -q 'org.freedesktop.Secret.Service' || fail "introspect"
gdbus call --session --dest org.freedesktop.secrets --object-path /org/freedesktop/secrets \
    --method org.freedesktop.Secret.Service.ReadAlias default | grep -q collection/login || fail "alias"

echo "python keyring and secretstorage"
python3 - <<'PY'
import keyring, secretstorage
from keyring.backends import SecretService
assert isinstance(keyring.get_keyring(), SecretService.Keyring), keyring.get_keyring()
keyring.set_password("svc", "u", "pw1")
assert keyring.get_password("svc", "u") == "pw1"
keyring.delete_password("svc", "u")
assert keyring.get_password("svc", "u") is None
conn = secretstorage.dbus_init()
coll = secretstorage.get_default_collection(conn)
coll.create_item("py", {"app": "py"}, b"bytes\x00ok", replace=True)
items = list(coll.search_items({"app": "py"}))
assert len(items) == 1 and items[0].get_secret() == b"bytes\x00ok"
PY

echo "git-credential-libsecret"
printf 'protocol=https\nhost=example.com\nusername=me\npassword=tok\n\n' | "$GIT_LIBSECRET" store
printf 'protocol=https\nhost=example.com\n\n' | "$GIT_LIBSECRET" get | grep -q '^password=tok$' || fail "git get"

echo "another app: allowed once asked (answer always)"
printf 'shared' | secret-tool store --label='Shared' app shared
python3 -c 'import keyring' # warm
python3 - <<'PY'
import secretstorage
conn = secretstorage.dbus_init()
coll = secretstorage.get_default_collection(conn)
item = next(coll.search_items({"app": "shared"}))
assert item.is_locked(), "another app's secret starts locked"
assert item.unlock() is False, "unlock went through a prompt"
assert item.get_secret() == b"shared"
PY

echo "libsecret's prompt path: secret-tool reads what python stored"
python3 - <<'PY'
import secretstorage
conn = secretstorage.dbus_init()
coll = secretstorage.get_default_collection(conn)
coll.create_item("from python", {"made": "py"}, b"pysecret", replace=True)
PY
[ "$(secret-tool lookup made py)" = pysecret ] || fail "secret-tool through a prompt"

echo "after a Lock call: secret-tool unlocks through a prompt"
gdbus call --session --dest org.freedesktop.secrets --object-path /org/freedesktop/secrets \
    --method org.freedesktop.Secret.Service.Lock "['/org/freedesktop/secrets/collection/login']" >/dev/null
[ "$(secret-tool lookup made py)" = pysecret ] || fail "lookup after lock"

echo "a 2 MiB call doesn't take the service down"
python3 - <<'PY' || true
import secretstorage
conn = secretstorage.dbus_init()
coll = secretstorage.get_default_collection(conn)
try:
    coll.create_item("huge", {"huge": "1"}, b"x" * (2 << 20))
except Exception as e:
    print("refused as expected:", type(e).__name__)
PY
[ "$(secret-tool lookup made py)" = pysecret ] || fail "the service died on a huge call"

echo "another app: denied"
start deny
printf 'mine' | secret-tool store --label='Mine' app mine
python3 - <<'PY'
import secretstorage
conn = secretstorage.dbus_init()
coll = secretstorage.get_default_collection(conn)
item = next(coll.search_items({"app": "mine"}))
assert item.unlock() is True, "dismissed"
assert item.is_locked()
try:
    item.get_secret()
    raise SystemExit("read a denied secret")
except secretstorage.exceptions.LockedException:
    pass
PY

echo "all passed"
