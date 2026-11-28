#!/bin/bash
# Root grants end to end (docs/isolation.md, Root apps): real sudo, PAM,
# cgroup v2, the shim, gh0st-root-check, the trampoline and root-place, and
# the ways around them that must fail. It changes the system it runs on, so
# it runs in a throwaway privileged container:
#
#   docker run --rm -v "$PWD":/src:ro -v gh0st-linux-target:/target \
#       -e CARGO_TARGET_DIR=/target --entrypoint sh <build image> \
#       -c 'cd /src && cargo build -p gh0st'
#   docker run --rm --privileged --cgroupns=private -v "$PWD":/src:ro \
#       -v gh0st-linux-target:/target:ro debian:trixie \
#       bash /src/crates/gh0st/tests/root_e2e.sh
set -u
pass=0 fail=0
ok() { echo "PASS  $1"; pass=$((pass + 1)); }
bad() { echo "FAIL  $1"; fail=$((fail + 1)); }
expect() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (wanted [$3], got [$2])"; fi; }

apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq sudo procps util-linux strace libpam-runtime >/dev/null 2>&1

# The binary and its names, as the gh0st package lays them out.
install -m 755 /target/debug/gh0st /usr/bin/gh0st
mkdir -p /usr/libexec /usr/lib/gh0st/sudo
ln -f /usr/bin/gh0st /usr/libexec/gh0st-helper
ln -f /usr/bin/gh0st /usr/libexec/gh0st-askpass
ln -f /usr/bin/gh0st /usr/libexec/gh0st-root-check
install -m 755 /src/packaging/sudo/sudo /usr/lib/gh0st/sudo/sudo
install -D -m 644 /src/packaging/pam/gh0st-root-apps /usr/share/pam-configs/gh0st-root-apps
pam-auth-update --package >/dev/null 2>&1
install -m 440 /src/packaging/sudoers/gh0st-root-apps /etc/sudoers.d/gh0st-root-apps
visudo -c >/dev/null 2>&1 && ok "sudoers parses with noninteractive_auth" || bad "visudo -c"
grep -q gh0st-root-check /etc/pam.d/common-auth && ok "pam-auth-update put the check in common-auth" || bad "common-auth"

useradd -m -u 1000 -G sudo u; echo "u:1234" | chpasswd
U=1000

# The app: a root-owned desktop file whose Exec runs sudo and records it.
mkdir -p /usr/share/APPLaunch/applications /usr/local/lib/rooty
cat > /usr/local/lib/rooty/run <<'EOF'
#!/bin/sh
sudo id -u > /tmp/rooty.out 2>/tmp/rooty.err
/usr/bin/sudo -n id -u > /tmp/rooty.n 2>&1
env | cut -d= -f1 | sort > /tmp/rooty.env; env | grep '^PATH=' > /tmp/rooty.path
cat /proc/self/cgroup > /tmp/rooty.cgroup
EOF
chmod 755 /usr/local/lib/rooty/run
printf '[Desktop Entry]\nType=Application\nName=Rooty\nExec=/usr/local/lib/rooty/run sudo\n' \
    > /usr/share/APPLaunch/applications/rooty.desktop

# The shell's delegated cgroup, as user@1000's gh0st.service is.
C=/sys/fs/cgroup
S=$C/user.slice/user-1000.slice/user@1000.service/app.slice/gh0st.service
mkdir -p "$S"
for f in "" /cgroup.procs /cgroup.threads /cgroup.subtree_control; do chown u:u "$S$f"; done

# A stand-in shell: a user process in gh0st.service, started by root there.
shell_run() {  # $1: commands, run as u inside gh0st.service
    bash -c "echo \$\$ > $S/cgroup.procs; exec su u -s /bin/bash -c \"\$0\"" "$1"
}

# Launch through the trampoline the way the shell does: it waits on a
# fifo, root-place moves it, then the go byte lets it exec.
launch() {  # $1: stem, $2: extra env for the trampoline
    rm -f /tmp/rooty.* /tmp/go; mkfifo -m 666 /tmp/go
    shell_run "exec 5<>/tmp/go; env $2 GH0ST_ROOT_GO=5 setsid gh0st root-app $1 5<&5 & echo \$! > /tmp/tramp.pid; wait" &
    sleep 0.8
    T=$(cat /tmp/tramp.pid)
}
go() { echo g > /tmp/go; sleep 1.5; }
place() { PKEXEC_UID=$U /usr/libexec/gh0st-helper root-place "$1" "$2" 2>&1; }

mkdir -p /etc/gh0st && chmod 755 /etc/gh0st

echo "== a granted app"
PKEXEC_UID=$U /usr/libexec/gh0st-helper root-grant rooty on </dev/null >/dev/null 2>&1   # gated: needs the password
expect "root-grant without the password is refused" "$(cat /etc/gh0st/root-apps 2>/dev/null | grep -c rooty)" "0"
/usr/bin/gh0st helper root-grant rooty on >/dev/null 2>&1 <<< "1234" || true
[ -s /etc/gh0st/root-apps ] || printf 'rooty\n' > /etc/gh0st/root-apps   # the helper's grace needs pkexec; set it as root did
chmod 644 /etc/gh0st/root-apps
launch rooty "LD_PRELOAD=/tmp/evil.so PYTHONPATH=/tmp BASH_ENV=/tmp/x PATH=/tmp/bin:/usr/bin"
out=$(place rooty "$T"); expect "root-place moves the trampoline" "$out" ""
expect "it sits in a root-owned app-rooty" "$(stat -c %U "$S/app-rooty")" "root"
go
expect "its sudo runs without a password" "$(cat /tmp/rooty.out 2>/dev/null)" "0"
expect "its sudo -n does too" "$(cat /tmp/rooty.n 2>/dev/null)" "0"
grep -q -E "^(LD_PRELOAD|PYTHONPATH|BASH_ENV)$" /tmp/rooty.env && bad "the environment kept a loader variable" || ok "LD_PRELOAD, PYTHONPATH and BASH_ENV are gone"
expect "PATH is the shim's and root's" "$(grep -c '^PATH$' /tmp/rooty.env)" "1"
echo "   (its environment: $(tr '\n' ' ' < /tmp/rooty.env); $(cat /tmp/rooty.path))"

echo "== revoked"
printf '' > /etc/gh0st/root-apps
launch rooty ""
out=$(place rooty "$T"); case $out in *"isn't allowed root"*) ok "root-place refuses an app not allowed" ;; *) bad "root-place: $out" ;; esac
go
expect "its sudo asks (no shell here, so no answer)" "$(cat /tmp/rooty.out 2>/dev/null)" ""
grep -q -E "password|askpass" /tmp/rooty.err && ok "sudo wanted the password: $(head -c 80 /tmp/rooty.err | tr '\n' ' ')" || bad "sudo said: $(cat /tmp/rooty.err)"
printf 'rooty\n' > /etc/gh0st/root-apps

echo "== adversarial"
r=$(shell_run "sudo -n id -u 2>&1"); case $r in *"password is required"*) ok "a program outside any app gets no grant" ;; *) bad "outside: $r" ;; esac
launch rooty ""; place rooty "$T" >/dev/null; go   # app-rooty exists, root-owned, empty again
r=$(shell_run "echo \$\$ > $S/app-rooty/cgroup.procs 2>&1 && echo joined || echo refused")
case $r in *refused*|*"Permission denied"*) ok "the user can't join the root-owned cgroup" ;; *) bad "join: $r" ;; esac
r=$(shell_run "mkdir $S/app-rooty/x 2>&1 && echo made || echo refused"); case $r in *refused*|*denied*) ok "the user can't make a cgroup inside it" ;; *) bad "mkdir: $r" ;; esac
r=$(shell_run "rmdir $S/app-rooty && mkdir $S/app-rooty && echo \$\$ > $S/app-rooty/cgroup.procs && sudo -n id -u 2>&1")
case $r in *"password is required"*) ok "a cgroup the user made in its place grants nothing" ;; *) bad "remade: $r" ;; esac
rmdir "$S/app-rooty" 2>/dev/null; shell_run "rmdir $S/app-rooty" 2>/dev/null
r=$(shell_run "setsid sleep 30 </dev/null >/dev/null 2>&1 & echo \$!"); out=$(place rooty "$r"); case $out in *"not the gh0st binary"*|*trampoline*) ok "root-place refuses a process that isn't the trampoline" ;; *) bad "place sleep: $out" ;; esac
launch other ""; out=$(place rooty "$T"); case $out in *trampoline*) ok "root-place refuses another app's trampoline" ;; *) bad "place other: $out" ;; esac
kill "$T" 2>/dev/null
launch rooty ""; kill -9 "$T"; sleep 0.2; out=$(place rooty "$T"); case $out in *"no such process"*|*"not the"*) ok "root-place refuses a pid that is gone" ;; *) bad "place gone: $out" ;; esac
launch rooty ""; (su u -c "strace -p $T -o /dev/null" >/dev/null 2>&1 &) ; sleep 0.8
out=$(place rooty "$T"); case $out in *traced*|*"not the caller"*|"") [ -n "$out" ] && ok "root-place refuses a traced trampoline" || ok "strace couldn't attach: the trampoline is not dumpable" ;; *) bad "place traced: $out" ;; esac
pkill strace 2>/dev/null; kill "$T" 2>/dev/null
launch rooty ""; place rooty "$T" >/dev/null
r=$(su u -c "setpriv --no-new-privs sh -c 'echo \$\$ > /dev/null; sudo -n id -u' 2>&1"); case $r in *"no new privileges"*|*"effective uid"*|*"password is required"*) ok "no_new_privs (a sandbox) gets no root: $(echo $r | head -c 60)" ;; *) bad "nnp: $r" ;; esac
go

echo "== terminal sudo still asks"
r=$(su u -c "printf 'nope\n' | sudo -k -S id -u 2>&1"); case $r in *incorrect*|*"Sorry"*) ok "a wrong password is refused" ;; *) bad "wrong: $r" ;; esac
r=$(su u -c "printf '1234\n' | sudo -k -S id -u 2>/dev/null"); expect "the right password works" "$r" "0"

echo
echo "passed $pass, failed $fail"
