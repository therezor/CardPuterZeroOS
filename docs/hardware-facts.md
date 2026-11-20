# Hardware facts

Captured from a device, not guessed. Nothing below is measured yet: each item
is a check for M1, with what the code assumes today.

| Check | Assumed now | Where it matters |
|---|---|---|
| Panel device and bpp | `/dev/fb_lcd`, 16 bpp RGB565, no byte swap | `cpz-ui/src/fb.rs` |
| Rows pushed per flush | Only pages written to the fbdev mapping | flush cost, animation |
| Boot splash `fb_load` | May hold DRM until the stock binary path runs | first frame of cpz |
| Keyboard device | `/dev/input/by-path/platform-3f804000.i2c-event` | `cpz-core/src/evdev.rs` |
| Kernel key repeat | Ignored; cpz repeats at 500/50 ms | `keys.rs` |
| Sym map file | `/usr/share/keymaps/tca8418_keypad_m5stack_keymap.map` | `keys.rs` |
| Backlight | `/sys/class/backlight/backlight`, writable by UID 1000 | `backlight.rs` |
| Backlight GPIO | stock drives M5IOE1 line 9 low at boot; cpz does not yet | cold boot |
| Battery | a `type=Battery` supply under `/sys/class/power_supply` | `battery.rs` |
| Battery uevents | sent on capacity change? if not, add a slow timer | idle wakeups |
| `wpctl`, `pw-play`, `bluetoothctl` | installed | volume, sounds, Settings |
| Root actions | time and ADB need root; reboot, power off, 5V rails do not | Settings |

## Baseline (stock APPLaunch)

To fill in with `scripts/measure.sh M5CardputerZero-APPLaunch`.
