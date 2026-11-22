#!/bin/bash
# mmdebstrap customize hook: runs in the container with the finished root
# at $1, before it is packed. Env: REPORTS, PARTUUID_*, ROOTS.
set -euo pipefail
root=$1
ch() { chroot "$root" "$@"; }

# The build's apt sources and dpkg options go. cpzeroos-base ships the
# device's own.
rm -f "$root/etc/apt/sources.list" "$root"/etc/apt/apt.conf.d/99mmdebstrap \
    "$root"/etc/dpkg/dpkg.cfg.d/99mmdebstrap

echo zero > "$root/etc/hostname"
printf '127.0.0.1\tlocalhost\n127.0.1.1\tzero\n::1\t\tlocalhost ip6-localhost ip6-loopback\n' > "$root/etc/hosts"

# No user: first-boot setup (docs/setup.md) makes UID 1000 on the device.
# Its marker starts gh0st-setup.service and keeps the shell off until setup
# hands over.
install -d -m 755 "$root/var/lib/cpzeroos"
install -d -m 700 "$root/var/lib/cpzeroos/setup-home"
: > "$root/var/lib/cpzeroos/setup"

cat > "$root/etc/fstab" <<FSTAB
PARTUUID=$PARTUUID_BOOT  /boot/firmware  vfat  noatime,fmask=0022,dmask=0022  0  2
PARTUUID=$PARTUUID_ROOT  /               ext4  noatime,commit=60,errors=remount-ro  0  1
PARTUUID=$PARTUUID_DATA  /home           ext4  noatime,commit=60,nodev,nosuid,x-systemd.growfs  0  2
FSTAB

# Only the units in our presets stay enabled. The tmpfiles timer is static,
# so it is masked: /tmp is a tmpfs and the daily wakeup buys nothing.
# systemd-backlight would restore the brightness saved at shutdown, often 0
# after the screen timed out; the driver starts it lit and gh0st restores
# the user's level. Stock masks it too.
ch systemctl preset-all >/dev/null 2>&1
ch systemctl --global preset-all >/dev/null 2>&1
# NetworkManager.service lists wait-online in Also=, so its preset turns it
# back on. Nothing here waits for the network. The dispatcher stays: it is
# started over D-Bus per event, and NetworkManager logs a failure without it.
ch systemctl disable -q NetworkManager-wait-online.service
ch systemctl mask -q systemd-tmpfiles-clean.timer 'systemd-backlight@backlight:backlight.service'

# mmdebstrap marks its whole base set manual. Keep that for what the image
# asked for and Debian's required base; the rest are dependencies, so apt
# may remove them once nothing needs them.
keep=$( { tr , '\n' <<< "$ROOTS"
          ch dpkg-query -W -f='${Package} ${Priority} ${Essential}\n' | awk '$2 == "required" || $3 == "yes" {print $1}'
        } | sort -u)
auto=$(ch apt-mark showmanual | sort | comm -23 - <(echo "$keep"))
[ -z "$auto" ] || ch apt-mark auto $auto > /dev/null

# Per-device identity is made on the device, never baked in.
rm -f "$root"/etc/ssh/ssh_host_*
: > "$root/etc/machine-id"
rm -f "$root/var/lib/systemd/random-seed" "$root/var/lib/dbus/machine-id"
find "$root/var/log" -type f -delete
rm -f "$root"/var/cache/debconf/*-old "$root"/var/lib/dpkg/*-old

mkdir -p "$REPORTS"
ch dpkg-query -W -f='${Package}\t${Version}\t${Installed-Size}\n' | sort > "$REPORTS/packages.tsv"
ch apt-mark showmanual | sort > "$REPORTS/manual.txt"
ch dpkg-query -W -f='${Package}\t${Priority}\t${Essential}\n' | sort > "$REPORTS/priority.tsv"
ch systemctl list-unit-files --state=enabled,masked --no-legend --no-pager | awk '{print $1, $2}' | sort > "$REPORTS/units-system.txt"
ch systemctl --global list-unit-files --state=enabled --no-legend --no-pager | awk '{print $1, $2}' | sort > "$REPORTS/units-user.txt" || true
