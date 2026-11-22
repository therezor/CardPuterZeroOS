#!/bin/bash
# First-boot smoke test of the dev image in QEMU's raspi3b, a Pi 3 close to
# the CM0. No panel, keyboard, Wi-Fi or VideoCore there, so it proves the
# boot path only: kernel without initramfs, root by PARTUUID, the data
# partition growing, zram, SSH host keys, and UID 1000's session. QEMU's
# serial console stays silent with this device tree, so the test reads the
# dev image's persistent journal off the card file afterwards.
set -uo pipefail
build=$WORK/build/image-dev
s=$WORK/smoke
[ -f "$build/cpzeroos-dev.img" ] || { echo "no dev image: run os/build.sh image dev"; exit 1; }
rm -rf "$s"
mkdir -p "$s/tree/j"
cp --sparse=always "$build/cpzeroos-dev.img" "$s/sd.img"
truncate -s 4G "$s/sd.img"    # QEMU wants a power of two; p3 grows into it
secs=${SMOKE_SECS:-600}
echo "booting for ${secs}s (TCG emulation is slow)"
timeout "$secs" qemu-system-aarch64 -M raspi3b -m 1G \
    -kernel "$build/boot/kernel8.img" -dtb "$build/boot/bcm2710-rpi-3-b.dtb" \
    -drive file="$s/sd.img",format=raw,if=sd -display none -monitor none -serial null -no-reboot \
    -append "root=PARTUUID=$(. "$OS/image/layout.conf"; echo "${DISK_ID#0x}-02") rootfstype=ext4 rootwait fsck.repair=yes" \
    < /dev/null > "$s/qemu.log" 2>&1

# Read the journal with the image's own journalctl.
tar -xf "$build/rootfs.tar" -C "$s/tree" ./usr ./etc
for l in bin sbin lib; do ln -sf "usr/$l" "$s/tree/$l"; done
start=$(sfdisk -d "$s/sd.img" | awk -F'[=,]' '/sd.img2/ {print $2 + 0}')
dd if="$s/sd.img" of="$s/root.ext4" bs=512 skip="$start" count=$((3072 * 2048)) status=none
debugfs -R "rdump /var/log/journal $s/tree/j" "$s/root.ext4" 2>/dev/null
d=$(cd "$s/tree" && dirname "$(find j -name system.journal | head -1)")
chroot "$s/tree" journalctl -D "/$d" -o short-monotonic --no-pager > "$s/journal.txt" 2>&1
u=$(cd "$s/tree" && find j -name 'user-1000*.journal' | head -1)
[ -n "$u" ] && chroot "$s/tree" journalctl --file "/$u" -o short-monotonic --no-pager >> "$s/journal.txt" 2>&1

fails=0
expect() {
    if grep -qE "$2" "$s/journal.txt"; then printf 'ok    %s\n' "$1"; else printf 'FAIL  %s\n' "$1"; fails=$((fails + 1)); fi
}
expect "kernel mounted root without an initramfs" 'EXT4-fs \(mmcblk0p2\): mounted filesystem'
expect "data partition grown" 'Finished cpzeroos-grow-data.service'
expect "/home filesystem grown" 'Successfully resized "/home"'
expect "zram swap active" 'Activated swap dev-zram0.swap'
expect "SSH host keys made" 'ssh-keygen: generating new host keys'
expect "sshd listening" 'Server listening on'
expect "UID 1000 session started" 'Started user@1000.service'
expect "system reached multi-user" 'Reached target multi-user.target'
# QEMU has no Wi-Fi, and NetworkManager logs a profile only when it fails
# to load one. Joining the network is a device test.
if [ -n "${CPZ_DEV_WIFI_SSID:-}" ]; then
    if grep -qE 'NetworkManager.*keyfile.*(warn|fail|error)|NetworkManager.*<(warn|error)>.*system-connections' "$s/journal.txt"; then
        printf 'FAIL  NetworkManager rejected the dev Wi-Fi profile\n'; fails=$((fails + 1))
    else
        printf 'ok    NetworkManager took the dev Wi-Fi profile without complaint\n'
    fi
fi
# Failures QEMU causes (no VideoCore, no keyboard) are expected; any other
# failed unit is not.
other=$(grep -E 'Failed to start|failed with result' "$s/journal.txt" | grep -vE 'gh0st.service' | head -5)
if [ -z "$other" ]; then printf 'ok    no failed units beyond gh0st (no keyboard in QEMU)\n'; else printf 'FAIL  failed units:\n%s\n' "$other"; fails=$((fails + 1)); fi
if grep -q "Can't create dir" "$s/journal.txt"; then printf 'FAIL  home not writable\n'; fails=$((fails + 1)); else printf 'ok    home writable\n'; fi
echo
grep -E 'Startup finished in .*\(kernel\)' "$s/journal.txt" | head -1 | sed 's/.*Startup/QEMU: startup/'
echo "journal: os/out/smoke-journal.txt"
cp "$s/journal.txt" /out/smoke-journal.txt
if [ "$fails" -eq 0 ]; then echo "smoke passed"; else echo "$fails smoke check(s) failed"; exit 1; fi
