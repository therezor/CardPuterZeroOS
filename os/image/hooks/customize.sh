#!/bin/bash
# mmdebstrap customize hook: runs in the container with the finished root
# at $1, before it is packed. Env: VARIANT, REPORTS, and CPZ_DEV_* for dev.
set -euo pipefail
root=$1
ch() { chroot "$root" "$@"; }

# The build's apt sources and dpkg options go. cpzeroos-base ships the
# device's own.
rm -f "$root/etc/apt/sources.list" "$root"/etc/apt/apt.conf.d/99mmdebstrap \
    "$root"/etc/dpkg/dpkg.cfg.d/99mmdebstrap

if [ "$VARIANT" = dev ]; then
    user=${CPZ_DEV_USER:-zero}
    host=${CPZ_DEV_HOSTNAME:-cpzero-dev}
else
    user=zero
    host=zero
fi
echo "$host" > "$root/etc/hostname"
printf '127.0.0.1\tlocalhost\n127.0.1.1\t%s\n::1\t\tlocalhost ip6-localhost ip6-loopback\n' \
    "$host" > "$root/etc/hosts"

# UID 1000 runs gh0st and the apps. Groups: sudo and netdev for the polkit
# rules, the rest for the panel, keyboard, audio, camera and Cap buses.
ch useradd -m -u 1000 -U -s /bin/bash \
    -G sudo,netdev,video,input,render,audio,gpio,i2c,spi,dialout,plugdev "$user"
if [ "$VARIANT" = dev ] && [ -n "${CPZ_DEV_PASSWORD_HASH:-}" ]; then
    ch usermod -p "$CPZ_DEV_PASSWORD_HASH" "$user"
else
    # Until first-boot setup (roadmap M4) sets one.
    ch passwd -q -l "$user"
fi
mkdir -p "$root/var/lib/systemd/linger"
touch "$root/var/lib/systemd/linger/$user"

cat > "$root/etc/fstab" <<FSTAB
PARTUUID=$PARTUUID_BOOT  /boot/firmware  vfat  noatime,fmask=0022,dmask=0022  0  2
PARTUUID=$PARTUUID_ROOT  /               ext4  noatime,commit=60,errors=remount-ro  0  1
PARTUUID=$PARTUUID_DATA  /home           ext4  noatime,commit=60,nodev,nosuid,x-systemd.growfs  0  2
FSTAB

if [ "$VARIANT" = dev ]; then
    # The serial console logs straight in: if Wi-Fi never comes up, the UART
    # is the only way in, and the account may have no password.
    d=$root/etc/systemd/system/serial-getty@.service.d
    mkdir -p "$d"
    cat > "$d/50-cpzeroos-dev.conf" <<GETTY
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin $user --noreset --noclear --keep-baud 115200,57600,38400,9600 - \${TERM}
GETTY
    home=$root/home/$user
    if [ -n "${CPZ_DEV_SSH_PUBKEY:-}" ]; then
        install -d -m 700 -o 1000 -g 1000 "$home/.ssh"
        printf '%s\n' "$CPZ_DEV_SSH_PUBKEY" > "$home/.ssh/authorized_keys"
        chown 1000:1000 "$home/.ssh/authorized_keys"
        chmod 600 "$home/.ssh/authorized_keys"
    fi
    if [ -n "${CPZ_DEV_WIFI_SSID:-}" ]; then
        f=$root/etc/NetworkManager/system-connections/dev-wifi.nmconnection
        install -d -m 700 "${f%/*}"
        # The permanent MAC, so the router hands out the address it knows.
        cat > "$f" <<NM
[connection]
id=dev-wifi
type=wifi
autoconnect=true

[wifi]
mode=infrastructure
ssid=$CPZ_DEV_WIFI_SSID
cloned-mac-address=permanent

[wifi-security]
key-mgmt=wpa-psk
psk=${CPZ_DEV_WIFI_PSK:-}

[ipv4]
method=auto

[ipv6]
method=auto
NM
        chmod 600 "$f"
    fi
fi

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
