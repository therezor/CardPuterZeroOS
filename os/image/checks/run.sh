#!/bin/bash
# Tier 1 checks on the last image built for a variant. Offline: they read
# the reports, the rootfs tar and the boot partition. Exit 1 on any FAIL.
set -uo pipefail
variant=$1
build=$WORK/build/image-$variant
R=$build/reports
M=$OS/image/manifest
V=$OS/image/variants/$variant
[ -f "$build/rootfs.tar" ] || { echo "no image for $variant: run os/build.sh image $variant"; exit 1; }
fails=0
ok() { printf 'ok    %s\n' "$*"; }
bad() { printf 'FAIL  %s\n' "$*"; fails=$((fails + 1)); }
words() { grep -hv '^#' "$@" 2>/dev/null | awk 'NF {print $1}'; }

# Packages: exactly the lock. A new dependency is a reviewed diff.
lock_lines "$R/packages.tsv" > "$build/packages.now"
lock=$V/packages.lock
if [ ! -f "$lock" ]; then
    bad "no $lock: run os/build.sh relock $variant"
elif diff -u <(grep -v '^#' "$lock") "$build/packages.now" > "$build/packages.diff"; then
    ok "$(wc -l < "$build/packages.now") packages match the lock"
else
    bad "packages differ from the lock (os/build.sh relock $variant accepts):"
    grep -E '^[+-][^+-]' "$build/packages.diff" | head -30
fi

# Deny list.
hits=
while read -r pat; do
    while read -r name; do
        # shellcheck disable=SC2053
        [[ $name == $pat ]] && hits="$hits $name"
    done < <(cut -f1 "$R/packages.tsv")
done < <(words "$M/deny.list" "$V/deny.list")
if [ -z "$hits" ]; then ok "nothing from the deny list"; else bad "denied packages:$hits"; fi

# Raspberry Pi builds only where lock.json pins them.
rpt=$(awk -F'\t' '$2 ~ /rpt/ && $1 !~ /^cpzeroos-/ {print $1}' "$R/packages.tsv" | sort)
allowed=$(jq -r '.rpt[].name' "$LOCK" | sort)
extra=$(comm -23 <(echo "$rpt") <(echo "$allowed"))
if [ -z "$extra" ]; then ok "rpt packages all pinned in lock.json"; else bad "unpinned rpt packages: $extra"; fi

# What apt marks manual is roots.list plus Debian's required base, which
# mmdebstrap marks manual.
want=$(words "$M/roots.list" "$V/roots.list" | sort)
base=$(awk -F'\t' '$2 == "required" || $3 == "yes" {print $1}' "$R/priority.tsv" | sort)
unasked=$(comm -23 "$R/manual.txt" <(sort -u <(echo "$want") <(echo "$base")) | tr '\n' ' ')
unmarked=$(comm -23 <(echo "$want") "$R/manual.txt" | tr '\n' ' ')
if [ -z "$unasked$unmarked" ]; then
    ok "manual packages are roots.list plus the required base"
else
    bad "manual packages differ from roots.list: extra [$unasked] missing [$unmarked]"
fi

# Enabled units.
for kind in system user; do
    exp=$V/units-$kind.expected
    if [ ! -f "$exp" ]; then
        bad "no $exp: run os/build.sh relock $variant"
    elif diff -u "$exp" "$R/units-$kind.txt" > "$build/units-$kind.diff"; then
        ok "$kind units as expected ($(wc -l < "$exp"))"
    else
        bad "$kind units differ:"
        grep -E '^[+-][^+-]' "$build/units-$kind.diff"
    fi
done
if grep -q '\.timer enabled' "$R/units-system.txt"; then bad "a timer is enabled"; else ok "no timers enabled"; fi

# Per-device identity is made on the device.
list=$build/rootfs.list
tar -tvf "$build/rootfs.tar" > "$list"
if grep -q 'etc/ssh/ssh_host_' "$list"; then bad "SSH host keys in the image"; else ok "no SSH host keys"; fi
if awk '$6 == "./etc/machine-id" && $3 != 0 {f = 1} END {exit !f}' "$list"; then
    bad "machine-id is not empty"
else
    ok "machine-id empty"
fi
if grep -q 'var/lib/systemd/random-seed' "$list"; then bad "random seed in the image"; else ok "no random seed"; fi
if grep -E 'var/lib/apt/lists/[^/]+_' "$list" | grep -q .; then bad "apt lists in the image"; else ok "no apt lists"; fi
if [ "$variant" = release ]; then
    if grep -q 'system-connections/.' "$list"; then bad "Wi-Fi profiles in a release"; else ok "no Wi-Fi profiles"; fi
fi

# Dev: what the CPZ_DEV_* secrets should have produced, when they are set.
if [ "$variant" = dev ]; then
    if grep -q 'serial-getty@.service.d/50-cpzeroos-dev.conf' "$list"; then
        ok "serial console logs in"
    else
        bad "no serial autologin drop-in"
    fi
    if [ -n "${CPZ_DEV_WIFI_SSID:-}" ]; then
        if grep -qE '^-rw------- (root/root|0/0) .*system-connections/dev-wifi.nmconnection$' "$list"; then
            ok "Wi-Fi profile present, 0600 root"
        else
            bad "Wi-Fi profile missing or not 0600 root"
        fi
    fi
    if [ -n "${CPZ_DEV_SSH_PUBKEY:-}" ] && [ "${CPZ_DEV_SETUP:-}" != 1 ]; then
        u=${CPZ_DEV_USER:-zero}
        if debugfs -R "stat /$u/.ssh/authorized_keys" "$build/data.ext4" 2>/dev/null |
            grep -qE 'Mode: +0600.*' && debugfs -R "stat /$u/.ssh/authorized_keys" "$build/data.ext4" 2>/dev/null |
            grep -qE 'User: +1000 '; then
            ok "authorized_keys present, 0600 UID 1000"
        else
            bad "authorized_keys missing or wrong mode or owner"
        fi
    fi
    if [ -n "${CPZ_DEV_WIFI_COUNTRY:-}" ]; then
        if grep -q "cfg80211.ieee80211_regdom=$CPZ_DEV_WIFI_COUNTRY" "$build/bootfs/cmdline.txt"; then
            ok "Wi-Fi country on the kernel command line"
        else
            bad "Wi-Fi country missing from cmdline.txt"
        fi
    fi
fi

# A release, or a dev image built with CPZ_DEV_SETUP=1, runs first-boot
# setup: its marker, no user yet, root locked. Otherwise UID 1000 exists and
# its home sits at the top of the data partition, which is /home.
if [ "$variant" = release ] || [ "${CPZ_DEV_SETUP:-}" = 1 ]; then
    if grep -qE ' \./var/lib/cpzeroos/setup$' "$list"; then ok "setup's marker present"; else bad "no setup marker"; fi
    if tar -xOf "$build/rootfs.tar" ./etc/passwd | awk -F: '$3 == 1000 {f = 1} END {exit !f}'; then
        bad "a UID 1000 user in an image that runs setup"
    else
        ok "no user: setup makes it"
    fi
    if tar -xOf "$build/rootfs.tar" ./etc/shadow | awk -F: '$1 == "root" && $2 !~ /^[!*]/ {f = 1} END {exit !f}'; then
        bad "root has a password"
    else
        ok "root locked"
    fi
    if [ -n "${CPZ_DEV_SSH_PUBKEY:-}" ]; then
        if grep -qE '^-rw------- (root/root|0/0) .*var/lib/cpzeroos/seed/authorized_keys$' "$list"; then
            ok "SSH key staged for setup, 0600 root"
        else
            bad "SSH key not staged for setup"
        fi
    fi
elif debugfs -R "ls -l /" "$build/data.ext4" 2>/dev/null | awk '$2 ~ /^4/ && $4 == 1000 {f = 1} END {exit !f}'; then
    ok "home directory at the root of the data partition"
else
    bad "no UID 1000 home at the root of the data partition"
fi

# Boot partition: every overlay config.txt names is there, and nothing for
# other boards.
mdir -b -i "$build/boot.vfat" ::/overlays > "$build/overlays.list"
missing=
for o in $(sed -n 's/^dtoverlay=\([^,]*\).*/\1/p' "$build/bootfs/config.txt"); do
    grep -qx "::/overlays/$o.dtbo" "$build/overlays.list" || missing="$missing $o"
done
if [ -z "$missing" ]; then ok "all overlays in config.txt present"; else bad "missing overlays:$missing"; fi
if mdir -b -i "$build/boot.vfat" ::/ | grep -qiE 'initramfs|2712|start4'; then
    bad "Pi 4/5 or initramfs files on the boot partition"
else
    ok "boot files are CM0 only"
fi

# The rest needs the tree: extract it once.
tree=$build/tree
rm -rf "$tree"
mkdir -p "$tree"
tar -xf "$build/rootfs.tar" -C "$tree"

for f in /usr/bin/pkexec /usr/bin/sudo /usr/sbin/unix_chkpwd; do
    [ -e "$tree$f" ] && stat -c '%A %U:%G %n' "$tree$f" | sed "s|$tree||"
done > "$build/modes.txt"
if grep -q '^-rwsr-xr-x root:root /usr/bin/pkexec' "$build/modes.txt" &&
   grep -q '^-rwsr-xr-x root:root /usr/bin/sudo' "$build/modes.txt" &&
   grep -q '^-rwxr-sr-x root:shadow /usr/sbin/unix_chkpwd' "$build/modes.txt"; then
    ok "setuid and setgid bits kept"
else
    bad "setuid or setgid bits wrong:"
    cat "$build/modes.txt"
fi
if [ "$(stat -c %i "$tree/usr/bin/gh0st")" = "$(stat -c %i "$tree/usr/libexec/gh0st-helper")" ]; then
    ok "gh0st-helper is a hard link to gh0st"
else
    bad "gh0st-helper is not a hard link to gh0st"
fi

# Commands our binaries run.
missing=
for c in $(words "$M/exec.list"); do
    chroot "$tree" sh -c "command -v $c" > /dev/null || missing="$missing $c"
done
if [ -z "$missing" ]; then ok "every command in exec.list resolves"; else bad "missing commands:$missing"; fi

# Stock app and AppStore libraries.
nf=$(chroot "$tree" sh -c 'for f in /usr/share/APPLaunch/bin/*; do ldd "$f" | sed "s|^|$f: |"; done' | grep 'not found')
if [ -z "$nf" ]; then ok "stock app libraries resolve"; else bad "unresolved libraries: $nf"; fi

# Drivers match the kernel.
kver=$(jq -r .kernel.release "$LOCK")
wrong=
for k in "$tree"/usr/lib/modules/"$kver"/extra/*.ko; do
    modinfo -F vermagic "$k" | grep -q "^$kver " || wrong="$wrong ${k##*/}"
done
if [ -z "$wrong" ]; then ok "driver vermagic is $kver"; else bad "wrong vermagic:$wrong"; fi
if depmod -n -e -b "$tree" -F "$tree/boot/System.map-$kver" "$kver" 2>&1 >/dev/null | grep -qi 'unknown symbol'; then
    bad "unresolved module symbols"
else
    ok "depmod finds no unknown symbols"
fi

# Budget.
if [ -f "$V/budget.conf" ]; then
    # shellcheck source=/dev/null
    . "$V/budget.conf"
    n=$(wc -l < "$R/packages.tsv")
    mb=$(awk -F'\t' '{s += $3} END {print int(s / 1024)}' "$R/packages.tsv")
    if [ "$n" -le "$MAX_PACKAGES" ] && [ "$mb" -le "$MAX_INSTALLED_MB" ]; then
        ok "budget: $n packages (max $MAX_PACKAGES), $mb MB (max $MAX_INSTALLED_MB)"
    else
        bad "over budget: $n packages (max $MAX_PACKAGES), $mb MB (max $MAX_INSTALLED_MB)"
    fi
fi

rm -rf "$tree"
echo
if [ "$fails" -eq 0 ]; then echo "all checks passed"; else echo "$fails check(s) failed"; exit 1; fi
