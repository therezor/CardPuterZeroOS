# Hardware facts

Captured from the device (CM0, `RZR-ZERO`, 2026-11-20) unless marked.

## Measured

- Debian 13 trixie, kernel `6.18.39+rpt-rpi-v8`, 352 MB RAM visible to Linux.
- CPU features: `fp asimd evtstrm crc32 cpuid`. No `aes` or `sha2`: building
  with `target-cpu=cortex-a53` alone gives SIGILL, hence
  `-C target-feature=-aes,-sha2` in `.cargo/config.toml`.
- Panel: `/dev/fb_lcd` -> `fb0`, `panel-mipi-dbid`, 16 bpp, 320x170, stride
  640, group `video`. DRM card1 is `cardputer-zero-internal` (SPI).
- Keyboard: `tca8418c` at `/dev/input/by-path/platform-3f804000.i2c-event`.
  The sym map file exists.
- Backlight: `/sys/class/backlight/backlight`, max 100, group `video` writable.
  Stock leaves it at 0 after its idle timeout, so the shell must turn it on.
- Battery: `bq27220-0`, `type=Battery`. It sends a `change` uevent when the
  capacity moves, so no battery timer is needed.
- LEDs: `ext_5v_out`, `grove_5v_out`, `ext_usb_gpio_fun`, `grove_fun`.
- `wpctl`, `pw-play`, `aplay`, `bluetoothctl`, `nmcli`, `timedatectl`,
  `gpioset` are installed. `fb_load.service` has exited by the time the
  launcher runs. LightDM and labwc run too (HDMI desktop).

## Idle, 60 s on home (`scripts/measure.sh`)

| | Stock APPLaunch 0.6.105 | Shell (first device run) |
|---|---|---|
| PSS | 9,642 kB | 1,304 kB (944 kB of it the fb mapping) |
| Threads | 14 | 1 |
| Context switches | 35,026 | 24 |
| CPU | 5,430 ms | 10 ms |
| First frame | not measured | 79 ms after start, icons decoded eagerly |

Both screens went dark during the minute.

## Still to check



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
