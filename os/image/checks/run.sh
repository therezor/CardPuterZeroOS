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
rpt=$(awk -F'\t' '$2 ~ /rpt/ && $1 !~ /^gh0st-/ {print $1}' "$R/packages.tsv" | sort)
allowed=$(jq -r '.rpt[].name' "$LOCK" | sort)
extra=$(comm -23 <(echo "$rpt") <(echo "$allowed"))
if [ -z "$extra" ]; then ok "rpt packages all pinned in lock.json"; else bad "unpinned rpt packages: $extra"; fi

# What lock.json publishes only for Store apps stays out of the image.
inside=$(comm -12 <(cut -f1 "$R/packages.tsv" | sort) <(jq -r '.rpt[] | select(.use == "repo") | .name' "$LOCK" | sort))
if [ -z "$inside" ]; then ok "no Store-only package in the image"; else bad "Store-only packages installed: $inside"; fi

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
# One timer: the daily update check, which the user can turn off.
if grep '\.timer enabled' "$R/units-system.txt" | grep -qv '^gh0st-update-daily\.timer '; then
    bad "a timer other than the daily update check is enabled"
else
    ok "only the update check's timer"
fi

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
if grep -q 'system-connections/.' "$list"; then bad "Wi-Fi profiles in the image"; else ok "no Wi-Fi profiles"; fi

# First-boot setup makes the user: its marker, no UID 1000 yet, root locked.
if grep -qE ' \./var/lib/gh0st/setup$' "$list"; then ok "setup's marker present"; else bad "no setup marker"; fi
if tar -xOf "$build/rootfs.tar" ./etc/passwd | awk -F: '$3 == 1000 {f = 1} END {exit !f}'; then
    bad "a UID 1000 user in the image"
else
    ok "no user: setup makes it"
fi
if tar -xOf "$build/rootfs.tar" ./etc/shadow | awk -F: '$1 == "root" && $2 !~ /^[!*]/ {f = 1} END {exit !f}'; then
    bad "root has a password"
else
    ok "root locked"
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

# Stock app libraries.
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

# USB Wi-Fi firmware: every name gh0st-wifi's drivers ask for is there.
# ath9k_htc asks for 1.4.0 and loads htc_9271.fw (1.3) when that's absent.
# mt76x0u names the PCIe chip's file too, and rtw88_8822c a wake-on-LAN one.
optional=" ath9k_htc/htc_9271-1.4.0.fw ath9k_htc/htc_7010-1.4.0.fw mediatek/mt7610e.bin rtw88/rtw8822c_wow_fw.bin "
fwdir=$tree/usr/lib/firmware
gone=
for m in $(words "$OS/packages/gh0st-wifi/drivers"); do
    ko=$(find "$tree/usr/lib/modules/$kver" -name "$m.ko*" | head -1)
    [ -n "$ko" ] || { gone="$gone $m(no module)"; continue; }
    for n in $(modinfo -F firmware "$ko"); do
        [[ $optional == *" $n "* ]] && continue
        found=
        for f in "$fwdir/updates/$n" "$fwdir/$n"; do
            for x in "" .xz .zst; do [ -e "$f$x" ] && found=1; done
        done
        [ -n "$found" ] || gone="$gone $m:$n"
    done
done
if [ -z "$gone" ]; then ok "USB Wi-Fi drivers find their firmware"; else bad "missing Wi-Fi firmware:$gone"; fi

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
