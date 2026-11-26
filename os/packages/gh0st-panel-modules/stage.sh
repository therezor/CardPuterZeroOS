#!/bin/bash
# Builds the M5 drivers and our panel driver against the pinned kernel
# headers, then installs an explicit file list into the staging root $1.
# M5's `make install` writes to absolute paths, so only `make all` runs.
set -euo pipefail
dest=$1
kdir=/usr/src/linux-headers-$KVER
build=$WORK/build/panel-modules
rm -rf "$build"
mkdir -p "$build"
cp -a "$CACHE/git/m5stack-linux-dtoverlays/modules" "$build/m5"
cp -a "$OS/kernel/panel-mipi-dbi-m" "$build/panel"
cp -a "$OS/kernel/gh0st-power" "$build/power"

mk() {
    make -s -C "$1" KERNELDIR="$kdir" ARCH=arm64 CC=gcc-14 CONFIG_CARDPUTERO_V0_5=y all
}
for d in CardputerZero st7789v-1.0 tca8418-1.0 m5ioe1-1.0 bq27220-1.0 es8389-1.0 imu-1.0; do
    mk "$build/m5/$d"
done
for d in panel power; do
    make -s -C "$build/$d" KERNELDIR="$kdir" ARCH=arm64 CC=gcc-14
done
dtc -@ -q -I dts -O dtb -o "$build/panel/gh0st-panel-te.dtbo" "$build/panel/gh0st-panel-te.dts"

m5=$build/m5
# M5's panel-mipi-dbi-m.ko and powerfail_suo.ko are left out: ours replace
# them.
modules="
$m5/CardputerZero/cardputerzero-audio.ko
$m5/CardputerZero/gpio-forwarder.ko
$m5/st7789v-1.0/st7789v_m5stack.ko
$m5/st7789v-1.0/pwm_bl_m5stack.ko
$m5/tca8418-1.0/tca8418_keypad_m5stack.ko
$m5/m5ioe1-1.0/m5ioe1.ko
$m5/bq27220-1.0/bq27xxx_battery.ko
$m5/bq27220-1.0/bq27xxx_battery_i2c.ko
$m5/es8389-1.0/es8389_m5stack.ko
$m5/es8389-1.0/simple-amplifier.ko
$m5/imu-1.0/bmc150_magn.ko
$m5/imu-1.0/bmc150_magn_bmi270.ko
$m5/imu-1.0/bmi270_core.ko
$m5/imu-1.0/bmi270_i2c.ko
$build/panel/panel-mipi-dbi-m.ko
$build/power/gh0st-power.ko
"
# The v5 overlay carries every on-board device; the others are what
# config.txt names on top of it.
overlays="
$m5/CardputerZero/cardputerzero-v5-overlay.dtbo
$m5/bq27220-1.0/bq27220_v5.dtbo
$m5/imu-1.0/bmi270_bmm150_overlay.dtbo
$build/panel/gh0st-panel-te.dtbo
"

extra=$dest/usr/lib/modules/$KVER/extra
install -d "$extra" "$dest/usr/lib/gh0st/overlays" "$dest/usr/lib/firmware" \
    "$dest/usr/lib/udev/rules.d" "$dest/usr/lib/tmpfiles.d" "$dest/usr/share/keymaps" \
    "$dest/usr/sbin"
for f in $modules; do
    v=$(modinfo -F vermagic "$f")
    case $v in
        "$KVER "*) ;;
        *) echo "error: $f vermagic '$v' is not $KVER" >&2; exit 1 ;;
    esac
    install -m 644 "$f" "$extra/"
done
# imu-1.0 copies its subdirectories' modules up a level; count those once.
built=$(find "$m5"/{CardputerZero,st7789v-1.0,tca8418-1.0,m5ioe1-1.0,bq27220-1.0,es8389-1.0,imu-1.0} \
    "$build/panel" "$build/power" -name '*.ko' | grep -v '/imu-1.0/[^/]*/' | wc -l)
listed=$(echo "$modules" | grep -c .)
# M5's panel and power-fail modules are built but not shipped.
[ "$built" -eq $((listed + 2)) ] || { echo "error: built $built modules, listed $listed" >&2; exit 1; }
for f in $overlays; do install -m 644 "$f" "$dest/usr/lib/gh0st/overlays/"; done

install -m 644 "$m5/st7789v-1.0/cardputerzero,st7789v_lcd.bin" "$m5/imu-1.0/bmi270-init-data.fw" \
    "$dest/usr/lib/firmware/"
install -m 644 "$m5/st7789v-1.0/99-cardputer-zero-lcd.rules" \
    "$m5/tca8418-1.0/99-cardputer-zero-keyboard.rules" \
    "$m5/CardputerZero/60-cardputerzero-leds.rules" "$dest/usr/lib/udev/rules.d/"
install -m 644 "$m5/CardputerZero/cardputerzero-leds.conf" "$dest/usr/lib/tmpfiles.d/"
install -m 644 "$m5/tca8418-1.0/tca8418_keypad_m5stack_keymap.map" "$dest/usr/share/keymaps/"
install -m 755 -s "$m5/st7789v-1.0/fb_load" "$dest/usr/sbin/"
