#!/bin/sh
# Writes a CPZeroOS image to an SD card on macOS. Destroys what is on it.
#   scripts/flash.sh os/out/cpzeroos-dev-YYYYMMDD.img.xz /dev/diskN
# Find N with `diskutil list`: the card is the external, physical disk.
set -eu
img=${1:?image .img.xz}
disk=${2:?disk, for example /dev/disk4}
case $disk in
    /dev/disk[0-9]*) ;;
    *) echo "expected /dev/diskN, got $disk" >&2; exit 1 ;;
esac
diskutil info "$disk" | grep -qE 'Device Location: +External|Removable Media: +Removable' || {
    echo "$disk is not an external or removable disk; refusing" >&2
    exit 1
}
diskutil info "$disk" | grep -E 'Device / Media Name|Disk Size'
printf 'Erase %s and write %s? Type yes: ' "$disk" "$(basename "$img")"
read -r answer
[ "$answer" = yes ] || exit 1
diskutil unmountDisk "$disk"
# The raw device is several times faster than the buffered one.
xz -dc "$img" | sudo dd of="${disk%disk*}rdisk${disk##*disk}" bs=4m
sync
diskutil eject "$disk"
echo "done: $img on $disk"
