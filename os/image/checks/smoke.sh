#!/bin/bash
# First-boot smoke test of the release image in QEMU's raspi3b, a Pi 3
# close to the CM0. No panel, keyboard, Wi-Fi or VideoCore there, so it
# proves the boot path only: kernel without initramfs, root by PARTUUID, the
# data partition growing, zram, and first-boot setup starting. QEMU's serial
# console stays silent with this device tree, so the test reads the journal
# off the card file afterwards.
set -uo pipefail
build=$WORK/build/image-release
s=$WORK/smoke
[ -f "$build/cpzeroos-release.img" ] || { echo "no image: run os/build.sh image release"; exit 1; }
# shellcheck source=/dev/null
. "$OS/image/layout.conf"
rm -rf "$s"
mkdir -p "$s/tree/j"
cp --sparse=always "$build/cpzeroos-release.img" "$s/sd.img"
# A power of two for QEMU; p3 grows into it. Larger cards stall QEMU's slow
# SD emulation behind ext4's inode table writes (SMOKE_CARD=64G to try).
truncate -s "${SMOKE_CARD:-8G}" "$s/sd.img"

# The image keeps no logs. The test's own copy gets a drop-in that keeps
# them on disk, synced every 10 s so the timeout's kill loses little.
start=$(( (ALIGN_MIB + BOOT_MIB) * 2048 ))
dd if="$s/sd.img" of="$s/root.ext4" bs=512 skip="$start" count=$((ROOT_MIB * 2048)) conv=sparse status=none
printf '[Journal]\nStorage=persistent\nSyncIntervalSec=10s\n' > "$s/60-smoke.conf"
debugfs -w -f - "$s/root.ext4" > /dev/null 2>&1 <<DEBUGFS
mkdir /etc/systemd/journald.conf.d
write $s/60-smoke.conf /etc/systemd/journald.conf.d/60-smoke.conf
DEBUGFS
debugfs -R "cat /etc/systemd/journald.conf.d/60-smoke.conf" "$s/root.ext4" 2>/dev/null | grep -q persistent ||
    { echo "could not add the journal drop-in"; exit 1; }
dd if="$s/root.ext4" of="$s/sd.img" bs=512 seek="$start" conv=notrunc,sparse status=none
secs=${SMOKE_SECS:-600}
echo "booting for ${secs}s (TCG emulation is slow)"
timeout "$secs" qemu-system-aarch64 -M raspi3b -m 1G \
    -kernel "$build/boot/kernel8.img" -dtb "$build/boot/bcm2710-rpi-3-b.dtb" \
    -drive file="$s/sd.img",format=raw,if=sd -display none -monitor none -serial null -no-reboot \
    -append "root=PARTUUID=${DISK_ID#0x}-02 rootfstype=ext4 rootwait fsck.repair=yes" \
    < /dev/null > "$s/qemu.log" 2>&1

# Read the journal with the image's own journalctl.
tar -xf "$build/rootfs.tar" -C "$s/tree" ./usr ./etc
for l in bin sbin lib; do ln -sf "usr/$l" "$s/tree/$l"; done
dd if="$s/sd.img" of="$s/root.ext4" bs=512 skip="$start" count=$((ROOT_MIB * 2048)) conv=sparse status=none
debugfs -R "rdump /var/log/journal $s/tree/j" "$s/root.ext4" 2>/dev/null
d=$(cd "$s/tree" && dirname "$(find j -name system.journal | head -1)")
chroot "$s/tree" journalctl -D "/$d" -o short-monotonic --no-pager > "$s/journal.txt" 2>&1

fails=0
expect() {
    if grep -qE "$2" "$s/journal.txt"; then printf 'ok    %s\n' "$1"; else printf 'FAIL  %s\n' "$1"; fails=$((fails + 1)); fi
}
expect "kernel mounted root without an initramfs" 'EXT4-fs \(mmcblk0p2\): mounted filesystem'
expect "data partition grown" 'Finished cpzeroos-grow-data.service'
expect "/home filesystem grown" 'Successfully resized "/home"'
expect "root checked" 'Finished systemd-fsck-root.service'
expect "/home checked" 'Finished systemd-fsck@dev-disk-by.*x2d03.service'
expect "/home mounted" 'Mounted home.mount'
if grep -qE 'EXT4-fs.*(corrupt|error)|Structure needs cleaning|emergency.target' "$s/journal.txt"; then
    printf 'FAIL  filesystem errors or emergency mode\n'; fails=$((fails + 1))
else
    printf 'ok    no filesystem errors, no emergency mode\n'
fi
expect "zram swap active" 'Activated swap dev-zram0.swap'
# Setup starts, and no user session until it hands over. QEMU has no
# keyboard, so setup itself fails and restarts.
expect "setup started" 'Started gh0st-setup.service'
if grep -q 'Started user@1000.service' "$s/journal.txt"; then
    printf 'FAIL  a user session before setup\n'; fails=$((fails + 1))
else
    printf 'ok    no user session before setup\n'
fi
if grep -q 'sshd.*Server listening' "$s/journal.txt"; then
    printf 'FAIL  sshd running: SSH is off until the user turns it on\n'; fails=$((fails + 1))
else
    printf 'ok    SSH off\n'
fi
expect "system reached multi-user" 'Reached target multi-user.target'
# Failures QEMU causes are expected: no keyboard for gh0st and setup, no
# panel for fb_load, which waits for it until its timeout. Any other failed
# unit is not.
other=$(grep -E 'Failed to start|failed with result' "$s/journal.txt" | grep -vE '(gh0st|gh0st-setup|fb_load).service' | head -5)
if [ -z "$other" ]; then printf 'ok    no failed units beyond what QEMU lacks\n'; else printf 'FAIL  failed units:\n%s\n' "$other"; fails=$((fails + 1)); fi
echo
grep -E 'Startup finished in .*\(kernel\)' "$s/journal.txt" | head -1 | sed 's/.*Startup/QEMU: startup/'
echo "journal: os/out/smoke-journal.txt"
cp "$s/journal.txt" /out/smoke-journal.txt
if [ "$fails" -eq 0 ]; then echo "smoke passed"; else echo "$fails smoke check(s) failed"; exit 1; fi
