# Hardware facts

Measured on the device unless marked.

## Measured

- Debian 13 trixie, kernel `6.18.39+rpt-rpi-v8`, 352 MB RAM visible to Linux
  at `gpu_mem=128`, 414 MB at `gpu_mem=64`.
- Bluetooth attaches in the kernel: `hci_uart_bcm` on `serial0` from the device
  tree finds the CYW4343A2. No `hciuart` or `pi-bluetooth`.
- The kernel has ext4 and the SD host drivers built in, and `CONFIG_LSM` is
  empty: root mounts without an initramfs, and no AppArmor.
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
  Fn+1..0 F1 to F10, Fn+O/P F11/F12, Fn+Bksp Delete.
- Fn+Esc keeps the Esc code: the reader sees Fn (389) down, Esc down, Esc
  up, Fn up. A tapped Fn gives the same sequence as a held one: the driver
  holds a one-shot Fn down until the next key comes up. Plain Esc comes alone.
  Measured with a raw evdev read.
- Ctrl is one-shot in the driver too: tapped, it stays down until the next
  key comes up, so Ctrl then C reads as Ctrl+C. Shift tapped twice stays
  down until tapped again; no Caps Lock code comes. Alt is a plain key.
- Fn then Alt reads as Alt with Fn held, and the next key comes plain: the
  Keyboard app's latch. Fn+Tab and Fn+Enter read as Tab and Enter with Fn
  held, and so do Fn+G and Fn+Y. Fn+J is SysRq. Sym then A reads as code 79
  (`;`).
- Fn+Q, W and E send 164 (play or pause), 165 (previous) and 163 (next).
  Nothing on the device acts on them.
- The only side button is power/boot. It sends no key event.
- The side switch to Slave drops the hub (Ethernet, USB-A, the other USB-C);
  back on Host the hub and the Ethernet chip come back in about 2 s. The
  chip (CoreChips `0fe6:9900`) has no address of its own and takes a random
  one at every plug. Its `enx` name followed it, and NetworkManager's stable
  wired address mixes in the name, so every plug got a new IP. The image
  names it `lan0` (`50-gh0st-lan.link`): the address and the lease now stay
  the same across plugs and boots.
- First boot of the Gh0st OS image (kernel `6.18.60+rpt-rpi-v8`, stock
  firmware), read off the card: userspace in 6.5 s, 9.3 s in all.
  vc4 takes DRM minor 0, the panel minor 1 at 8.5 s; `fb_load` waits for
  `/dev/dri/card1` and shows the logo at 9.0 s. The keyboard, battery,
  Wi-Fi (BCM43439) and Bluetooth (CYW4343A2) all probe. The RTC
  (`rtc-ds1307` at 1-0032) logs "oscillator failed": it had no time to keep.
  The panel logs one "Timeout waiting for TE signal" before the logo.
- The data partition grown from 256 MiB to a 64 GB card with 1 KiB blocks
  came up with corrupt group descriptors (group 4128) and `/home` would not
  mount. The image makes it with 4 KiB blocks. The cause
  was the card, not the block size: see "A fake card" below. Group 4128 sits
  about 32 GiB into p3, past the card's real capacity.
- **A fake card**. The 64 GB test card, sold as Sony,
  reports manfid 0x05, OEM 0x000c, name "asdfg", serial 0x12, made 07/2025.
  On it, ext4's backup superblocks read back right up to 18 GiB and as
  garbage at 33 GiB and 46 GiB, so its real capacity is between them. Data
  past that stays readable only while it is in the page cache: under memory
  pressure, the home folder (inode table at 55 GiB) came back as random bytes,
  "Inode checksum does not match", and every login failed. With root and
  `/home` on one partition, system files can land there too.
- Panel timing: ST7789V on SPI0 CS0. `spi-max-frequency` is 60 MHz, but the
  400 MHz core clock divides to 50 MHz, so a full frame takes about 17.4 ms.
  The panel refreshes at 60.5 Hz by default (TE period 16.5 ms). With
  FRCTRL2 0x15 it measures 50.6 Hz (19.76 ms). TE is high for 0.58 ms of each
  period (blanking).
- TE is wired to GPIO5 (M5's overlay has the line commented out). With
  TEON sent it pulses once per refresh.
- The dev device runs the official 2026-09-29 image, drivers from
  m5stack-linux-dtoverlays `7551854` (`/etc/cardputerzero-dtoverlays.commit`).
  Its M5 panel module is the newest M5 version: the sysfs reset is in, and
  there is no TE code (M5 turned TE off on 2026-09-23 "until timing is
  verified"). That file is unchanged through `a3fd3e8` (2026-11-19), which the
  official 2026-11-19 image and Gh0st OS build from. Our driver started from it.
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
  1, on home, discharging. So dark writes 1 and a black frame.
- Battery: `bq27220-0`, `type=Battery`, with no interrupt in the device tree.
  The kernel polls it every 360 s (`poll_interval`). Nothing tells the kernel
  about the charger: no expander input, USB event or log line follows the
  cable. Reads of `status` follow plug and unplug, and a read makes the driver
  send a `change` uevent when something moved. On stock, `wf-panel-pi` (the
  HDMI desktop's panel) reads it every 5 s, so most uevents come from that.
  The delay from the cable to `status` is not timed.
  gh0st reads it every 2 s while a bar is lit. With the screen dark and
  nothing reading it, the gauge's own poll still sent a `change` uevent
  5 min 14 s in. The shell watches uevents in the dark too; its
  low-battery countdown firing from one is not yet seen on the device.
- LEDs: `ext_5v_out`, `grove_5v_out`, `ext_usb_gpio_fun`, `grove_fun`.
- `wpctl`, `pw-play`, `aplay`, `bluetoothctl`, `nmcli`, `timedatectl`,
  `gpioset` are installed. `fb_load.service` has exited by the time the
  launcher runs. LightDM and labwc run too (HDMI desktop).

## Keyring and isolation (kernel `6.18.60+rpt-rpi-v8`)

- The shell's environment has `XDG_RUNTIME_DIR=/run/user/1000` and
  `DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus`; apps inherit it.
- dbus-daemon 1.16.2 gives `ProcessFD` in `GetConnectionCredentials`.
- `/sys/kernel/security/lsm` lists `capability` only: no Landlock, no Yama
  (`/proc/sys/kernel/yama` is missing). The shell and the keyring make
  themselves non-dumpable instead.
- `user.max_user_namespaces` is 1494. `unshare -Ur` and `unshare -Urm` work
  as UID 1000; inside, `/dev/fb_lcd` and `/dev/input` open and a tmpfs
  mounts.
- `gh0st.service` runs in `user@1000.service/app.slice/gh0st.service` with
  `Delegate=no`.
- A CoreChips USB Ethernet adapter (`r8152`, `0fe6:9900`) has no address of
  its own: it comes up as `enx4adbd2912288`, a locally administered MAC,
  with NetworkManager's "Wired connection 1". Its stable-id is now fixed
  (`50-gh0st.conf`) so the lease survives a replug.
- The keyring on the device (gh0st 11d399c): its daemon and `gh0st session`
  are one thread and about 490 kB PSS each, and woke 0 times in 20 s idle.
  `unix_chkpwd` takes 0.30 s. Argon2id (`examples/argon_bench`): 64 MiB
  three passes 1.50 s, two 1.03 s; 32 MiB two passes 0.51 s; 19 MiB two
  passes 0.30 s. opt-level 3 changes nothing: it is memory bound. With 32
  MiB, two passes, run beside `unix_chkpwd`: about 0.8 s from Enter on a
  prompt to the app holding its secret (0.95 s measured, less 0.12 s for
  the test's own key sender).
- Checked by hand there: open at start with the lock password off, an app
  storing and reading its own secret, a second app approved (always) and
  a third denied, a question lighting a dark screen, Show in the Keyring
  app after the password, Lock then a lookup that asks for it, a
  notification as a toast. Settings, Store, Terminal, Updates, Files,
  Camera, Weather and Music all start as before.

## Gh0st OS memory

On the release image after setup, idle on home, Wi-Fi up, read over SSH.

- 414 MB visible to Linux (`gpu_mem=64`; the 64 MB CMA pool counts, 58 MB of
  it free). About shows about 140 MB used, which is MemTotal minus
  MemAvailable. With an SSH session open it read 166 MB:

| Part | MB |
|---|---|
| Held back by MemAvailable: the high watermark, plus the low watermark of page cache and half the reclaimable slab | 53 |
| Programs (anon and shmem), about 5 MB of it the SSH session | 39 |
| Unreclaimable slab (kernfs, kmalloc, task structs) | 26 |
| vmalloc, 7.5 MB of it module code | 15 |
| Kernel stacks, page tables, per-CPU | 5 |
| Not in any counter: free pages in the per-CPU lists (10 to 20 MB), CMA in use (6 MB), driver pages | 32 |

- The watermarks come from `vm.min_free_kbytes = 16384`, the Raspberry Pi OS
  value: min 16 MB, low 20 MB, high 24 MB. The per-CPU free lists are sized
  from the low watermark: up to 5 MB a core, 19.8 MB in all at one read.
  MemFree leaves them out, so they count as used.
- Read twice after a 30 s settle: 16384 shows 152 and 156 MB used, 8192
  shows 129 MB twice. Under a 320 MB memory hog and 150 MB sent over Wi-Fi
  at 8192 there was no "page allocation failure", but brcmfmac logged
  SDIO read errors (CMD53 -5). Not tried at 16384, and the card above
  corrupted data in the same run, so 8192 is not proven safe.
- WirePlumber's v4l2, libcamera and MIDI monitors off: PipeWire 3.6 to
  1.9 MB private, WirePlumber unchanged (4.3 MB).
- Toggling SSH: `systemctl enable` or `disable` took 3.9 s, 2.6 s of it
  Debian's SysV sync (`update-rc.d`, Perl) and 1.25 s the manager reload.
  With `SYSTEMCTL_SKIP_SYSV=1` and `--no-reload`: 0.05 to 0.13 s. Start
  0.40 s, stop 0.07 s, `unix_chkpwd` on a yescrypt hash 0.30 s.
- PSS: NetworkManager 13.0 MB, WirePlumber 8.8, systemd 8.8, PipeWire 6.8,
  wpa_supplicant 6.8, systemd --user 4.4, udevd 3.8, polkitd 3.4, bluetoothd 3.0,
  logind 2.3, journald 2.2, (sd-pam) 2.0, timesyncd 2.0, the two dbus-daemons
  2.5, gh0st shell 1.8. 85 MB in all, 38 MB of it private.
- The camera stack loads at boot from `dtoverlay=imx219` with no camera app
  open: unicam, ISP, codec, legacy V4L2, MMAL and `vc_sm_cma`.
- No swap in use. Keys were made by the first SSH start in under a second.

## M5's Store

`gh0st-m5-appstore` 0.6.111+gh0st1 on the release image, opened from home
over Ethernet, read over SSH with `scripts/measure.sh` and `/proc`. The
baseline our Store is measured against (docs/store.md).

- **Open to list: about 100 s.** "Loading Artwork" then "Preparing Apps",
  62.5 s of CPU on the way. One app shows at a time.
- **Each open downloads 8.0 MB** (the interface's receive counter): the
  catalog again and all 179 icons. Its cache after one open is 11 MB: icons
  6.7 MB, the catalog 1.6 MB, kept debs 2.0 MB.
- **Memory on the list: 57 MB PSS**, 42 MB of it private anonymous, 65 MB
  RSS, 8 threads.
- **It never rests.** On the list with no key pressed, one thread runs a
  whole core: 5.1 s of CPU in every 5 s, about 600 context switches a
  second. Dark, the shell pauses it and the CPU stops.

## Our Store

`gh0st-store` built for the device, run as the user with `--sim` (frames to
PNG, no panel) against CI's catalog of 163 apps, keys from a script: scroll,
open an app, step the filters, search. Read from `/proc` over SSH. Not yet
through the shell on the panel: that needs the engine deployed as root.

- **Memory: 2.3 MB PSS**, 3.0 MB at its peak, one thread. The binary ran
  from tmpfs, so its 1.3 MB of pages count as shared memory; 1.0 MB is
  anonymous.
- **At rest: nothing.** 0 CPU ticks in 10 s, 0 wakeups in 5 s.
- **What a refresh downloads**, from the dry run of CI's mirror: the
  catalog, 74 KB (23 KB compressed); the descriptions, 257 KB (108 KB), and
  the icon pack, 532 KB, only when their hash changes. Opening the Store
  downloads nothing.

## The stock system

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
  `CLK_TCK` is 100.

## Dark current

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
- After a restart of Gh0st OS the Ethernet chip reads `power/control=auto`
  from the image's rule, the hub and root hub are suspended, and dwc2 fired
  no IRQ in 10 s.

## Sound of a paused app

Stock Music playing, then stopped with Fn+K, on Gh0st OS with the charger
in, so no current figures. Wakeups a second over 20 s of PipeWire's threads
(its and pipewire-pulse's data loops, pipewire-pulse's main loop) and
schedutil's `sugov` thread, which follow the screen less than the system
total does:

| State | Codec PCM | PipeWire and `sugov` | System total |
|---|---|---|---|
| Dark, no stream, PipeWire running | closed | 65 | 713 |
| Music stopped, its stream left open | RUNNING | 1,750 | 4,372 |
| The same stream on a null sink of quantum 8192 | closed | 100 | 830 |

The baseline was dark; the switcher showed for part of the other two.

- A stopped client's stream stays uncorked. pipewire-pulse goes on asking
  it for data at its own quantum (Music: `node.latency` 300/48000), so the
  sink, the codec and the I2S DMA keep running.
- A null sink with no other setting suspends the codec, but the graph still
  cycles at the app's quantum, driven by the Dummy-Driver. With
  `node.force-quantum=8192` on the sink it cycles about six times a second.
  A stream played meanwhile kept its own quantum on the ALSA sink (2048).
- Recorder opens a capture stream as soon as it starts, for its level
  meter. Stopped, it keeps the ADC running, and PipeWire and `sugov` wake
  about 500 times a second. Moved by hand to a null sink's monitor, the ADC
  suspends and they wake about 120 times.
- Unloading the null sink moves its streams back to the default device.
  Music played on, and the capture stream went back to the microphone.
- A move with pactl saves the new sink as the app's target in WirePlumber's
  `stream-properties`. WirePlumber restores a saved target only while a
  node of that name exists, so a target naming a gone sink falls back to
  the default. `state.restore-props=false` on a sink keeps the sink itself
  out of the file.
- The key is the first of `media.role`, `application.id`,
  `application.name`, so other apps can share it. While a parked sink
  exists, a new stream with that key would be restored onto it and play
  silent (`state-stream.lua`, not run). The image sets
  `node.stream.restore-target = false`: targets are still written, never
  restored.
- `pactl suspend-sink` on the null sink does nothing while a stream is
  linked to it.
- `gh0st audio park` took 0.25 s; with no audio library mapped in the group
  it returns in 19 ms without calling pactl.
- From source, not measured: the AW8737A amplifier's enable (GPIO24)
  follows the stream. M5's `simple-amplifier.c` sets it in DAPM's
  POST_PMU and clears it in PRE_PMD.

## System sounds through `pacat`

Raw clips piped into `pacat --raw --latency-msec=30`, timed by recording
the board's own microphone, with the charger in.

- A held stream plays a clip as soon as it is written, an 18 ms tick
  included: nothing waits for the buffer to fill.
- With the sink suspended, a new stream's PCM was PREPARED after 169 ms
  and RUNNING after 192 ms; in a later run the first clip came about
  120 ms after its write. Three cold streams in a row played their first
  clip whole: the same length and energy as the same clip played warm
  600 ms later. No lead of silence is needed.
- One `pacat` per clip instead adds 100 to 130 ms to every clip, the cold
  one included, and holds nothing open.
- A held stream with nothing to play still runs the graph: about 1,970
  wakeups a second (`sugov` 1,006, the data loops 475 and 235,
  pipewire-pulse 259), as a stopped app's open stream does.
- Closing its stdin ends `pacat` in 79 ms, and the sink suspends.
- `pipewire-pulse` takes 7.9 MB PSS once a pulse client has started it.

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

The model rests on TE being high during blanking and on the scan
following the send order (MADCTL ML).

## Paused apps and the keyboard

Measured with `python3` reading the evdev node directly.

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

## Boot files and root checks

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
  fourth wrong try exits 12 (`locked 5`).
- `/etc/hosts` has two `127.0.1.1` lines, `cardputerzero` and `RZR-ZERO`.
  `iw` is not installed. `/usr/share/zoneinfo/iso3166.tab` is.
- Swap on stock, the helper run as root over SSH (not yet through pkexec
  from Settings): `gh0st-helper swap 256` writes `FixedSizeMiB=256` to
  `/etc/rpi/swap.conf.d/90-gh0st-swap.conf`. After a restart zram0 is
  268435456 bytes. `swap 0` (`Mechanism=none`) boots with no swap and
  deletes `/var/swap`; `swap 256` after it brings both back. No failed units
  at any step; 900 is refused (cap 828 with 414 MB RAM).
- Stock runs zstd (the kernel's default; rpi-swap sets none) and swappiness 60
  (rpi-swap sets only `vm.page-cluster=0`, in `/run/sysctl.d/70-rpi-swap.conf`).
  `swap-comp lz4` boots zram0 with `[lz4]`, rpi-swap's size kept.
  `swappiness 150` applies at once and holds after a restart. Same root run
  over SSH.

## Wi-Fi access point (stock card)

Read on the stock Raspberry Pi OS card (pi-gen 2026-09-29), kernel
6.18.39+rpt-rpi-v8, brcmfmac firmware BCM43439/5 7.95.75, NetworkManager
1.52.1. Clients: an iPhone and a POCO X7 Pro. Design in `hotspot.md`.

- `iw list` gives two combinations: `#{managed} <= 2, P2P..., #channels <= 2`,
  and `#{managed} <= 1, #{AP} <= 1, #{P2P-client} <= 1, #{P2P-device} <= 1,
  #channels <= 1`. As UID 1000, `iw event`, `iw dev`, `station dump` and
  `SIOCGIWMODE` all work.
- **Alongside** (`uap0`, `type __ap`, wlan0 joined on channel 5): the access
  point starts on wlan0's channel and wlan0 stays up. But:
  - NetworkManager offers WPA2/WPA3 for `key-mgmt=wpa-psk`. The iPhone tried
    SAE and the firmware failed to send the auth replies
    (`brcmf_cyw_mgmt_tx: TX Auth frame operation is timedout`). With
    `pmf=disable` (WPA2 only) it joined.
  - `nmcli device wifi rescan` cut pings to a client from 0% to 56% lost over
    20 s, and the client was disconnected once.
  - Re-activating the access point logged `brcmf_cfg80211_stop_ap: setting AP
    mode failed -52` and dropped wlan0. wlan0 then could not rejoin its WPA3
    network while the access point ran ("No PMK set for PTK derivation",
    `brcmf_cyw_external_auth` failures), until the radio went off and on.
  - Stopping the access point alone left wlan0 up (the same -52 is logged).
- **On wlan0 alone**: WPA2/WPA3 mixed and WPA3-only (SAE, PMF required) both
  worked. Both phones joined over SAE, with a USB cable up for NAT (that a
  page loaded on them is not yet confirmed). NetworkManager picked channel 11
  when none was set. The
  BSSID was a locally administered address with `cloned-mac-address=stable`.
  `SIOCGIWMODE` on wlan0 reads 3 (master); `/proc/net/wireless` then has no
  wlan0 line.
- Shared mode sets `net.ipv4.ip_forward=1` and adds `table ip
  nm-shared-<iface>`. dnsmasq listens on 10.42.0.1:53 and 0.0.0.0:67 and uses
  4 MB RSS. Its lease file is root-only and holds client names.
- Battery `current_now`, screen on, cable in: 337 mA with the access point
  off, 375 mA on with two idle clients.
- Timing as UID 1000: `pkexec gh0st-helper` 150 to 300 ms, an nmcli call
  about 150 ms, `hotspot on` 2.4 s.

## Wi-Fi firmware at boot (release image)

- The BCM43439's firmware download can fail at boot: `brcmf_sdio_verifymemory:
  Downloaded RAM image is corrupted, block offset is 0, len is 780`, then
  `dongle nvram file download failed` and `mmc1: Controller never released
  inhibit bit(s)`. No `wlan0`, and NetworkManager reports `WIFI-HW missing`.
  The Bluetooth patch download to the same chip ran at the same moment.
- brcmfmac then unbinds both SDIO functions (`mmc1:0001:1` and `:2`, SDIO ID
  `04B4:BD3D`, function 1 has `OF_NAME=wifi`). Binding them again, or
  reloading the module, loads the firmware and `wlan0` comes up. How often it
  fails is not counted yet. `gh0st-wifi` binds them again on the unbind
  event, twice a boot at most.
- `iw phy` on the built-in chip lists IBSS, managed, AP and the P2P modes: no
  monitor (firmware 7.95.75).

## Camera (`gh0st-camd --probe`)

- IMX219 at `/base/soc/i2c0mux/i2c@1/imx219@10`, libcamera v0.7.2+rpt20260817.
  Pixel array 3280x2464. Modes: 640x480, 1640x1232, 1920x1080, 3280x2464.
- `Rotation` is 180: the sensor sits upside down, and libcamera flips it.
- The ISP gives RGB565 at the preview's own size, 226x170 (stride 480) and
  302x170 (stride 608). Nothing is scaled or converted on the CPU.
- `ScalerCrop` max is 3280x2464 at (0, 0) in the binned mode and
  1920x1080 at (680, 692) in the 1080p crop.
- CMA: with libcamera's defaults (four raw buffers) an 8 MP still fails to
  allocate. Two fit but drop 1 to 5 frames in 5 s at 1080p30. Three
  (`camd/vc4.yaml`) do neither. `CmaFree` reads near zero
  at idle because the page cache borrows CMA; it moves out on demand.
- 8 MP still: 575 ms from the preview to the frame, JPEG 501 ms at q95
  (2.2 MB) and 391 ms at q85 (1.0 MB), preview back in about 170 ms.
  A 2x still (1640x1232): 367 ms, JPEG 159 ms.
- Preview start 57 to 64 ms, first frame 23 to 118 ms.
- The sensor runs 1.2x slower than libcamera's model of it: frames came
  40.0 ms apart while `FrameDuration` said 33.3, so 30 fps gave 25, and a
  15 fps cap gave 12.5. The ratio measured 1.200 to 1.202 in both video
  modes. gh0st-camd scales every time it sets or reads by it
  (`GH0ST_CAMD_CLOCK` overrides).
- In video mode the ISP's second output gives YUV420, not RGB565, at
  302x170: the helper converts it.
- Video, 5 s each, with the clock fixed: 1080p (the 1920x1080 crop) 149
  frames, no gaps, 12.0 Mb/s; 720p (binned, scaled) 149 frames. CMA free
  while recording 1080p: about 17 MB. Files decode clean in ffprobe: High
  profile, level 4.1, BT.709 limited range.

## Files' tools (Debian trixie arm64 container)

Checked in a container with trixie's packages, not on a device.

- curl 8.14.1 lists `ftp ftps http https sftp scp smb smbs` among its
  protocols. That SMB can't list a folder is from curl's documentation, not
  tried here.
- OpenSSH 10.0p1 (`openssh-client 1:10.0p1-7+deb13u4`) with
  `SSH_ASKPASS_REQUIRE=force`, no tty and no `DISPLAY` (`setsid`, stdin from
  `/dev/null`): the new-host question, fingerprint included, and then the
  password both go to the askpass program, one call each.
  `StrictHostKeyChecking=ask` writes the key to `UserKnownHostsFile`.
- `/usr/lib/openssh/sftp-server` is there (openssh-sftp-server).
- bsdtar 3.7.4 (libarchive-tools) listed a zip, a 7z, an iso, and tars with
  xz, zstd and bzip2. rar was not tried. It adds two packages,
  `libarchive-tools` and `libarchive13t64`; its other libraries are on the
  image already.
- `bsdtar -xf` of a tar holding `../escaped` refused the entry ("Path
  contains '..'"), wrote nothing outside the target, and exited 1.

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
| Text: first screen of a 1 GB file | under 150 ms | `gh0st-text` bench (`examples/bench.rs`) |
| Text: PSS with a 1 GB file open | under 12 MB | `scripts/measure.sh gh0st-text` |
| Text: keystroke to flush | under 4 ms, one text row flushed | `GH0ST_LOG=debug` |
| Text: index and find slices | 8 ms at most each | `gh0st-text` bench |
| `EVIOCGRAB` while an app holds the grab | fails with `EBUSY` | dark over an app |
| Dark current | `current_now` lower with the terminal sent behind and the backlight off than on home | dark state |
| Restore-target off | a new stream with a parked app's key plays on the default device | `50-gh0st-streams.conf` |
| System sounds' current | lit home with sounds on draws no more than with them off, past the 2 s hold | `gh0st-shell/src/sound.rs` |
| A parked app's current | about 35 to 40 mA less than a stopped app with its stream open, as `cz-battery-saver` measured for the stock launcher | `gh0st audio` |
| Grace window | the helper's parent under pkexec is Settings, so a second change within 5 minutes skips the prompt | helper auth |
| `gpu_mem=64` | `vcgencmd` reads `gpu=64M` after a restart; camera and splash still work | Advanced |
| `61-gh0st-cpu.rules` | runs after stock's rule, so the governor survives a restart | Advanced |
| Swap on the image | `zram-size` in `/etc/systemd/zram-generator.conf.d/90-gh0st-swap-size.conf` beats `50-gh0st.conf`; 0 leaves no zram0 | Advanced |
| Swap compression and swappiness on the image | the same drop-ins work without rpi-swap | Advanced |
| Argon2id time and memory | m=64 MiB, t=3, p=1 unlocks in about 1 s; tune to that (`vault.rs`) | keyring unlock |
| Keyring and session PSS, idle wakeups | zero wakeups at idle (`scripts/measure.sh "gh0st keyring"`) | memory budget |
| `PR_SET_DUMPABLE 0` | blocks `ptrace` and `/proc/<pid>/mem` from UID 1000 with `ptrace_scope=1` | keyring memory |
| NetworkManager secret agent | accepted from UID 1000 with linger and no logind session; `psk-flags 1` calls `SaveSecrets`; registering clears `NO_SECRETS` | Wi-Fi in the keyring |
| cgroup delegation | `Delegate=yes` on `gh0st.service` gives the shell a writable subtree | app identity |

## Stock system baseline (`scripts/measure.sh system`)

Read on the stock M5 image with gh0st installed, 30 s idle, screen on, over
SSH.

| | Stock |
|---|---|
| MemTotal / MemAvailable | 415 / 212 MB, 108 MB in zram swap |
| CMA | 64 MB, 11 MB free |
| Packages, `/usr` | 1699, 5.1 GB |
| Running services | 20, including LightDM, avahi, rpcbind, udisks2, cron, rtkit |
| Timers | 9, including apt-daily and rpi-zram-writeback |
| Listening | sshd 22, rpcbind 111, avahi 5353, two random UDP ports |
| Boot | 5.1 s kernel + 30.2 s userspace |
| IRQs on dwc2, mmc, spi | about 8,000 a second |
| Battery current | 349 mA |

## Baseline (stock APPLaunch)

To fill in with `scripts/measure.sh M5CardputerZero-APPLaunch`.
