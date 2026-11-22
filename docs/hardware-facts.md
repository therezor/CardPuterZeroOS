# Hardware facts

Measured on the device unless marked.

## Measured

- Debian 13 trixie, kernel `6.18.39+rpt-rpi-v8`, 352 MB RAM visible to Linux.
- CPU features: `fp asimd evtstrm crc32 cpuid`. No `aes` or `sha2`: building
  with `target-cpu=cortex-a53` alone gives SIGILL, hence
  `-C target-feature=-aes,-sha2` in `.cargo/config.toml`.
- Panel: `/dev/fb_lcd` -> `fb0`, `panel-mipi-dbid`, 16 bpp, 320x170, stride
  640, group `video`. DRM card1 is `cardputer-zero-internal` (SPI).
- Keyboard: `tca8418c` at `/dev/input/by-path/platform-3f804000.i2c-event`.
  The sym map file exists. Fn sends 389 (`KEY_DVD`) and Sym 383 (`KEY_CD`),
  not `KEY_FN`. The kernel driver applies their layers itself: Fn+F/Z/X/C
  arrive as Up/Left/Down/Right, Fn+K as Home (102), Fn+H as Help (138),
  Fn+G as SysRq, Fn+A/S/D as
  mute and volume, Fn+Y/U as brightness. Ctrl 29, Alt 56. The device tree
  below says Fn+J for SysRq and Fn+U/I for brightness: press them to settle it.
- The driver's layers are in the device tree (`tca8418@34`: `linux,keymap`,
  `keymap1` for Sym, `keymap2` for Fn). Fn changes every key but R, T, Y, G,
  V, Space, Enter, Tab, Esc and the modifiers: Fn+Q/W/E media keys,
  Fn+U/I brightness, Fn+J SysRq, Fn+L/M PgUp/PgDn, Fn+N End, Fn+B Insert,
  Fn+1..0 F1 to F10, Fn+O/P F11/F12, Fn+Bksp Delete. Read 2026-11-21.
- Fn+Esc keeps the Esc code: the reader sees Fn (389) down, Esc down, Esc
  up, Fn up. A tapped Fn gives the same sequence as a held one: the driver
  holds a one-shot Fn down until the next key comes up. Plain Esc comes alone.
  Measured 2026-11-21 with a raw evdev read.
- The only side button is power/boot. It sends no key event.
- Panel timing: ST7789V on SPI0 CS0. `spi-max-frequency` is 60 MHz, but the
  400 MHz core clock divides to 50 MHz, so a full frame takes about 17.4 ms.
  The panel refreshes at 60.5 Hz by default (TE period 16.5 ms). With
  FRCTRL2 0x15 it measures 50.6 Hz (19.76 ms). TE is high for 0.58 ms of each
  period (blanking).
- TE is wired to GPIO5 (M5's overlay has the line commented out). With
  TEON sent it pulses once per refresh.
- The init file sets MADCTL 0xA0 (MV|MY): the panel refreshes across the
  landscape rows. gh0st's driver (`os/kernel/panel-mipi-dbi-m`) sends along
  the scan lines with MADCTL 0x90 and runs the panel at 50.6 Hz.
  0x40 is upside down, 0x80 still tears, 0x90 tears least. Those MADCTL tests
  ran while the TE wait was broken (see Frame pacing), so redo them.
- Backlight: `/sys/class/backlight/backlight`, max 100, group `video` writable.
  Stock leaves it at 0 after its idle timeout, so the shell must turn it on.
  The driver is `m5stack,pwm-backlight` on M5IOE1's pwm-3 (50 us period,
  levels 0 to 5500 ns). At 0 it switches the PWM off, and the LED then runs
  at full: battery current 385 mA at 0, 381 mA at 90, 340 mA at 5, 336 mA at
  1, on home, discharging. So dark writes 1 and a black frame. Measured
  2026-11-22.
- Battery: `bq27220-0`, `type=Battery`, with no interrupt in the device tree.
  The kernel polls it every 360 s (`poll_interval`). Nothing tells the kernel
  about the charger: no expander input, USB event or log line follows the
  cable. Reads of `status` follow plug and unplug, and a read makes the driver
  send a `change` uevent when something moved. On stock, `wf-panel-pi` (the
  HDMI desktop's panel) reads it every 5 s, so most uevents come from that.
  Measured 2026-11-21. The delay from the cable to `status` is not timed.
  gh0st reads it every 2 s while a bar is lit.
- LEDs: `ext_5v_out`, `grove_5v_out`, `ext_usb_gpio_fun`, `grove_fun`.
- `wpctl`, `pw-play`, `aplay`, `bluetoothctl`, `nmcli`, `timedatectl`,
  `gpioset` are installed. `fb_load.service` has exited by the time the
  launcher runs. LightDM and labwc run too (HDMI desktop).

## The stock system, read 2026-11-20

Read without changing anything, on the stock M5 image with gh0st installed.

- 352 MB visible to Linux, 216 MB used and 93 MB in swap at idle.
- Swap is already zram: `rpi-swap` sets up `/dev/zram0` (352 MB, priority 100)
  from `/etc/rpi/swap.conf`, with a file on a loop device for writeback
  (`rpi-setup-loop@var-swap`) and `rpi-zram-writeback.timer`. That timer wakes
  the system and writes swap to the SD card.
  `config.txt` has `gpu_mem=128` and `dtoverlay=vc4-kms-v3d,cma-64`.
- Running: LightDM, avahi, rpcbind, nfs-blkmap, udisks2, accounts-daemon, cron,
  polkit, rtkit, wpa_supplicant, NetworkManager, bluetooth, timesyncd, sshd.
- Listening: sshd on 22, rpcbind on 111 (TCP and UDP), avahi on 5353, and two
  random UDP ports.
- memcg is off, though `CONFIG_MEMCG=y`. The firmware adds
  `cgroup_disable=memory` to the command line; it isn't in `cmdline.txt`. The
  Pi fix is `cgroup_enable=memory` in `cmdline.txt`.
- PSI is built in but off: there is no `/proc/pressure`. It needs `psi=1`.
- `cmdline.txt` sets `cfg80211.ieee80211_regdom=AE`.
- Kernel config: cgroup freezer, `CGROUP_BPF`, `NO_HZ_IDLE`, `HZ=250`. As modules:
  zram, zstd, Adiantum, ChaCha20, NHPoly1305 (with NEON), dm-crypt, nftables with
  `NFT_SOCKET`, WireGuard. Built in: fscrypt, seccomp.
- Missing: Landlock and Yama (the LSM list is `capability` only), suspend
  (`PM_SLEEP` is off), and a PSCI CPU idle driver.
- `/proc/PID/task/TID/children` is there (`CONFIG_PROC_CHILDREN`), so the
  switcher's figures walk the shell's own tree instead of all 167 processes.
  `CLK_TCK` is 100. Read 2026-11-21.

## Dark current (measured 2026-11-22)

The dev device on the stock image, shell dark (backlight 1, app paused),
Wi-Fi joined on the built-in `wlan0` (brcmfmac, SDIO), battery current
averaged over 10 to 20 s. Each change was reverted after its run.

| State | mA | USB IRQ/s |
|---|---|---|
| Dark, as shipped | 346 | 8,100 |
| Ethernet chip (`r8152`, no cable) `power/control=auto` | 348 | 8,600 |
| Same, and the TP-Link USB Wi-Fi dongle deauthorized | 214 | 0 |
| Wi-Fi radios blocked (rfkill) | 315 | 8,100 |
| Panel DISPOFF and SLPIN (debugfs `command`) | 339 | 8,100 |
| CPU 1 to 3 offline | 346 | 8,100 |

- The USB host fires about 8,100 IRQ/s while any device on the hub is
  awake. The Ethernet chip suspends on `auto`, but the dongle (2357:011e,
  `rtw88_8821au`, `supports_autosuspend=0`) never does, so the hub and dwc2
  stay up. With both asleep the whole bus suspends: 132 mA less.
- Wi-Fi off saves about 30 mA; a reconnect costs a burst over 400 mA.
- Panel sleep is within the noise (about 7 mA), and offlining cores saves
  nothing: idle cores already wait in WFI.
- Readings drift 20 to 60 mA after a radio comes back, so compare runs taken
  back to back.
- With the dongle unplugged, back to back: the Ethernet chip on `auto`
  takes dark from 305 to 212 mA with the bus suspended (0 IRQ/s); Wi-Fi off
  from 302 to 274 mA. Settings' USB power saving, run through pkexec as the
  user, gives the same 212 mA. Wi-Fi off in the dark at 1 min blocked the
  radio 61 s into the dark, through `nmcli` from the shell's unit. The wake
  turning it back on is not yet checked on the device.

## Idle, 60 s on home (`scripts/measure.sh`)

| | Stock APPLaunch 0.6.105 | Shell (first device run) |
|---|---|---|
| PSS | 9,642 kB | 1,304 kB (944 kB of it the fb mapping) |
| Threads | 14 | 1 |
| Context switches | 35,026 | 24 |
| CPU | 5,430 ms | 10 ms |
| First frame | not measured | 79 ms after start, icons decoded eagerly |

Both screens went dark during the minute.

## Flush timing

Clock chain: `spi-max-frequency` 60 MHz (`0x03938700` in the DT). The VPU clock is fixed at
400 MHz (`vcgencmd measure_clock core`, `clk_summary`; `enable_uart=1`, `core_freq=400`). The RPi
docs say `enable_uart=1` implies `core_freq=250`, but this device measures 400. `spi-bcm2835`
rounds the divider up to even: 400/8 = 50 MHz. Driver params: `frctrl=0x15` (50 Hz),
`madctl_native=0x90`, `te=Y`, `te_timeout_ms=50`.

The fb.rs path: `flush` copies changed spans into the fbdev mapping. Each first write to a
page faults (`fb_deferred_io_mkwrite`, about 2 µs). The kernel sends nothing until the deferred-I/O
work runs **50 ms later** (`fbdefio.delay = HZ / 20` in `drm_fbdev_dma.c`). It then damages the
range from the lowest to the highest dirty page, at full width.

| | Full frame (first frame, 170 rows) | Clock tick (11 rows changed) |
|---|---|---|
| Pages faulted | 27 | 3 |
| Copy into the mapping | 0.26 ms | under 0.1 ms |
| Wait for deferred I/O | 50.5 ms | 49.5 to 50.6 ms |
| Wait for TE | 7.1 ms | 1.1 ms |
| SPI transfer (`RAMWR`) | 17.56 ms for 108,800 B = 49.6 Mbit/s | 2.14 ms, about 20 rows sent |
| Damage work total | 24.96 ms | 3.48 to 3.50 ms |
| Write to glass | 75.5 ms | about 53 ms |

- The SPI rate confirms 50 MHz. A full frame (17.6 ms) fits the 20 ms refresh with 2.4 ms to spare.
- A 4 KB page is 6.4 rows, so 11 changed rows sent about 20. Two changes far apart send every row
  between them.
- Before the fixes below, the 50 ms delay capped fbdev at about 13 to 19 updates a second.

### Fixes, measured (12 arrow-key presses, each a 7-frame highlight animation)

| | Before | `fsync()` in fb.rs | + driver `fbdev_delay=1` |
|---|---|---|---|
| Draw | 0.5 ms | 0.5 ms | 0.5 ms |
| Copy into the mapping (+ fsync) | 0.3 to 0.5 ms | 0.5 to 0.6 ms | 0.5 to 0.6 ms |
| `fsync()` call | none | 0.09 ms median, 2.9 ms max | 0.1 ms median, 4.0 ms max |
| First write to first frame on glass | 60.8 ms | 9.5 ms | 9.8 ms |
| Animation frames sent | 2 to 3 of 7 | 7 of 7 | 7 of 7 |
| Key frame to last frame on glass | 175 ms | 126 ms | 127 ms |

- **Draw is 0.5 ms.** The 21 ms seen on clock ticks happens between the wake and the draw
  (`wake to draw` in the log), in the work the tick does before drawing. It isn't draw or flush.
- **`fsync()` doesn't block the loop.** `fb_deferred_io_fsync` runs the deferred-I/O work now, but
  that work only queues `damage_work`. The SPI send happens in that worker after `fsync()` returns.
- **Driver `fbdev_delay`:** it defaults to 1 jiffy (`CONFIG_HZ=250`, so 4 ms). This helps apps
  that write `/dev/fb_lcd` without `fsync()`, such as stock apps. A test writer without `fsync()`
  measured 5.1 to 10.2 ms from write to send (median 6.6 ms), against 50 ms before. The cost is more
  write faults and SPI sends for apps that draw nonstop.
- One `Timeout waiting for TE signal` per boot, on the first frame. It was there before these
  changes too.
- **Tearing margin left alone.** 66.7 MHz (400 / 6) is above the DT's 60 MHz cap and the
  ST7789V's rated write clock. For more margin, slow the refresh with FRCTRL2 instead.

## Frame pacing

The first TE wait never waited. The driver disabled the TE IRQ between frames.
`pinctrl-bcm2835` has no `irq_disable`, so the disable was lazy. The next edge
was marked pending, and `enable_irq` replayed it from a tasklet 36 µs later.
Every send started at a random refresh phase. The "Wait for TE" times in the
table above are those replays, not real waits. The 39.9 fps full-frame rate was
the send time, not a panel limit.

The second cost was `drm_fb_swab`. It loaded each pixel from the
write-combined GEM buffer: 6.7 ms for a full frame.

What `os/kernel/panel-mipi-dbi-m` does now:
- **TE.** The TE IRQ timestamps both edges while frames flow. It learns the
  period (19.76 ms) and the blanking time (0.58 ms). The IRQ is unlazy, so a
  stale edge is dropped. It turns itself off after 20 edges without a send. Idle
  on home: `te_on 0`, and the IRQ count stays flat.
- **Send time.** A send starts where it can't cross the scan. That is before the
  scan reaches its first line (lead), or late enough that the scan passes its
  last line first (trail). Otherwise it sleeps until trail. Margin:
  `te_margin_us` (300). The wire time comes from measured sends: 163 ns/byte.
- **Fast copy.** Each row is memcpy'd out of the GEM buffer, then swapped and
  transposed in one pass. Prep went from 7.2 ms to 1.33 ms per full frame.
- **Present.** `GH0ST_FBIO_PRESENT` (`_IO('F', 0xC0)`) returns once the
  mapping's writes are on the wire. Stock fbdev answers ENOTTY.
- **Stats.** `/sys/bus/spi/devices/spi0.0/te_stats` shows period, blanking,
  wire speed, and lead/trail/sleep counts.

gh0st waits on `present` after each animation frame and draws the next one at
once, so animations lock to the refresh.

| | Before | Now |
|---|---|---|
| Key-press glide (partial rows) | about 21 ms timer, sends not synced | 77 of 94 sends one refresh apart (19.76 ms) |
| Full screen, back to back | 39.9 fps, random phase | 50.8 fps: 153 frames in 3 s, every gap 19 to 20 ms |
| Full frame: prep / RAMWR / total | 7.2 / 17.6 / 25.0 ms | 1.33 / 17.56 / 19.21 ms |

The other 17 key-press gaps are the first two sends of a glide: one leads in a
refresh, the next trails in the same refresh. The panel still shows one new
frame per refresh.

By eye: a blind A/B (`scripts/ab-test.sh`, 5 rounds, scan sync `te=0`
against `te=1`) picked the sync every round. That's 5 of 5, a 1 in 32 chance
if guessing. Whether any tear remains is not settled ("hard to tell, looks a
lot better"). The model rests on TE being high during blanking and on the scan
following the send order (MADCTL ML).

## Paused apps and the keyboard

Measured 2026-11-20 with `python3` reading the evdev node directly.

- A client that stops reading keeps queuing keys. A paused app replays them
  on SIGCONT.
- Writing `EV_MSC/MSC_SCAN` + `SYN_REPORT` pairs to the node reaches every
  client. 64 pairs overflow a stopped client's queue: it then starts with
  `SYN_DROPPED` and holds none of the earlier keys. The keypad declares
  `MSC_SCAN` (`B: MSC=10`), so the core does not drop them. UID 1000 can
  open the node read-write (group `input`).
- `passwd` run as the user reads the current and new passwords from piped
  stdin (a wrong current password fails with exit 10).
- nmcli as UID 1000: `wifi.scan` and `enable-disable-wifi` yes;
  `network-control` and `settings.modify.*` need auth (no active session).
- `rfkill` and `evtest` are not installed. `/dev/rfkill` carries an ACL for
  the user while a session is open. `pkexec` is installed.
- `/etc/systemd/journald.conf.d/persist.conf` sets `Storage=persistent`.

## Boot files and root checks, read 2026-11-21

- `/boot/firmware` is vfat (`fmask=0022`), so UID 1000 can read `config.txt`
  and `cmdline.txt`. A copy of `config.txt` is the test fixture in
  `gh0st-core/src/testdata`. Its `gpu_mem=128` sits at the top, before any
  section. The `[cm4]` and `[cm5]` sections don't apply to the CM0.
- `vcgencmd get_mem gpu` works as UID 1000 and prints `gpu=128M`.
  `/proc/cmdline` carries `cmdline.txt` after the firmware's own arguments.
- Password hashes are yescrypt (`$y$`). `/usr/sbin/unix_chkpwd` (libpam
  1.7.0, setgid shadow) exits 7 on a wrong password, with the password
  NUL-terminated or not.
- cpufreq: one `policy0` for all four cores, driver `cpufreq-dt`, 600 to
  1000 MHz, governors conservative, ondemand, userspace, powersave,
  performance and schedutil. Stock's udev rule `60-ondemand-governor.rules`
  (raspberrypi-sys-mods) sets ondemand when the CPUs appear.
- pkexec passes stdin through: a wrong password piped to
  `pkexec gh0st-helper gpu-mem 64` exits 11, no stdin exits 10, and the
  fourth wrong try exits 12 (`locked 5`). Measured 2026-11-21.
- `/etc/hosts` has two `127.0.1.1` lines, `cardputerzero` and `RZR-ZERO`.
  `iw` is not installed. `/usr/share/zoneinfo/iso3166.tab` is.
- Swap on stock, the helper run as root over SSH (not yet through pkexec
  from Settings): `gh0st-helper swap 256` writes `FixedSizeMiB=256` to
  `/etc/rpi/swap.conf.d/90-gh0st-swap.conf`. After a restart zram0 is
  268435456 bytes. `swap 0` (`Mechanism=none`) boots with no swap and
  deletes `/var/swap`; `swap 256` after it brings both back. No failed units
  at any step; 900 is refused (cap 828 with 414 MB RAM). Measured 2026-11-21.
- Stock runs zstd (the kernel's default; rpi-swap sets none) and swappiness 60
  (rpi-swap sets only `vm.page-cluster=0`, in `/run/sysctl.d/70-rpi-swap.conf`).
  `swap-comp lz4` boots zram0 with `[lz4]`, rpi-swap's size kept.
  `swappiness 150` applies at once and holds after a restart. Same root run
  over SSH. Measured 2026-11-21.

## Still to check



| Check | Assumed now | Where it matters |
|---|---|---|
| Panel device and bpp | `/dev/fb_lcd`, 16 bpp RGB565, no byte swap | `gh0st-ui/src/fb.rs` |
| Boot splash `fb_load` | May hold DRM until the stock binary path runs | first frame of gh0st |
| Keyboard device | `/dev/input/by-path/platform-3f804000.i2c-event` | `gh0st-core/src/evdev.rs` |
| Kernel key repeat | Ignored; gh0st repeats at 500/50 ms | `keys.rs` |
| Sym map file | `/usr/share/keymaps/tca8418_keypad_m5stack_keymap.map` | `keys.rs` |
| Backlight | `/sys/class/backlight/backlight`, writable by UID 1000 | `backlight.rs` |
| Backlight GPIO | stock drives M5IOE1 line 9 low at boot; gh0st does not yet | cold boot |
| Remaining tearing | check by eye with `scripts/tear-test.sh`; if it tears, widen `te_margin_us` or slow the refresh (FRCTRL2). No faster SPI clock (above the DT cap and the ST7789V rating) | driver |
| Battery | a `type=Battery` supply under `/sys/class/power_supply` | `battery.rs` |
| `wpctl`, `pw-play`, `bluetoothctl` | installed | volume, sounds, Settings |
| Root actions | time and ADB need root; reboot, power off, 5V rails do not | Settings |
| `unix_chkpwd` right password | exits 0 with `password\0` on stdin, run by root | helper auth |
| `unix_chkpwd` as the user | exits 0 for the user's own password and 7 (`PAM_AUTH_ERR`) for a wrong one, run by UID 1000 (setgid `shadow`). Any other exit lets the user in and stops the lock asking | lock screen |
| `passwd -S` as the user | prints `<user> P ...` for the user's own account, `NP` or `L` otherwise | lock screen |
| `EVIOCGRAB` while an app holds the grab | fails with `EBUSY` | dark over an app |
| Dark current | `current_now` lower with the terminal sent behind and the backlight off than on home | dark state |
| Grace window | the helper's parent under pkexec is Settings, so a second change within 5 minutes skips the prompt | helper auth |
| `gpu_mem=64` | `vcgencmd` reads `gpu=64M` after a restart; camera and splash still work | Advanced |
| `61-gh0st-cpu.rules` | runs after stock's rule, so the governor survives a restart | Advanced |
| Swap on the image | `zram-size` in `/etc/systemd/zram-generator.conf.d/90-gh0st-swap-size.conf` beats `50-cpzeroos.conf`; 0 leaves no zram0 | Advanced |
| Swap compression and swappiness on the image | the same drop-ins work without rpi-swap | Advanced |

## Baseline (stock APPLaunch)

To fill in with `scripts/measure.sh M5CardputerZero-APPLaunch`.
