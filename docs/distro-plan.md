# CPZeroOS: base distro and optimization plan

Status: research, 2026-11-20. Nothing here has been measured on a device yet. Figures marked
**measured** come from community repos (linked); everything else is a target to measure.

## 1. Recommendation

**Base: Debian 13 "trixie" arm64, built additively with `mmdebstrap --variant=minbase`,
plus the Raspberry Pi `rpt` 6.18 kernel/firmware and M5Stack's CardputerZero drivers,
with the kernel pinned.** Camera included, `gpu_mem=64`. No desktop in the base image (desktop
mode comes later as an app). No cloud-init; logging and network services are off by default.

Why not another distro:

- **Drivers:** the LCD, keyboard, audio codec, IO expander and fuel gauge are all out-of-tree
  M5 modules (`m5stack/m5stack-linux-dtoverlays`), proven only against the RPi `6.18.x+rpt-rpi-v8`
  kernel. Any distro must carry that kernel + those modules.
- **Apps:** the whole CardputerZero app ecosystem (M5 apt repo, AppStore, GitHub releases) is
  **arm64 glibc `.deb`s built for trixie** that depend on `libpulse0`, `libcamera0.8`,
  `ffmpeg`, etc. Buildroot/Alpine (musl) break all of them.
- **Precedent:** `on-my-machine-works/kali-on-cardputerzero` already builds a working image this
  way (mmdebstrap minbase + RPi kernel/firmware + M5 modules), so the path is proven.

Buildroot stays an option for a later "appliance" variant **only if** we drop stock-app/AppStore
compatibility (open question 1). It would have the lowest RAM use and fastest boot, but it needs
the M5 modules ported and every app rebuilt.

| Option | Drivers | Stock `.deb` apps | RAM/boot floor | Effort |
|---|---|---|---|---|
| Stock M5 image (RPi OS **desktop**, pi-gen stage 0–4) | ✅ | ✅ | worst | none |
| M5 pi-gen "lite" profile (stage 0–2), then strip | ✅ | ✅ | medium; still pulls cloud-init, avahi, rpi-connect, build tools | low |
| **Debian minbase via mmdebstrap (recommended)** | ✅ (pinned kernel) | ✅ | low | medium |
| DietPi | ✅ (RPi kernel), but M5 modules must be added | ✅ | low | medium, plus its own tooling and update channel |
| Buildroot / Yocto | port M5 modules | ❌ (rebuild everything) | lowest | high |
| Alpine / postmarketOS | port M5 modules | ❌ (musl) | lowest | high |

## 2. Hardware facts that matter to the OS

| Item | Detail |
|---|---|
| SoC | RPi CM0 Lite: RP3A0 (BCM2837/BCM2710A1), 4× Cortex-A53 @ 1.0 GHz, aarch64 |
| RAM | **512 MB** LPDDR2 (PoP), shared with the VideoCore GPU |
| Storage | microSD only (no eMMC) |
| LCD | ST7789V3 320×170 RGB565 on SPI0 CS0 @ ≤60 MHz, DC=GPIO25, TE=GPIO5 (unused in DT), backlight = PWM on M5IOE1. DRM driver `panel-mipi-dbi-m` → `/dev/dri/cardputer-zero-internal` + fbdev emulation `/dev/fb_lcd` |
| Keyboard | TCA8418 @ 0x34 (I2C1), IRQ GPIO27 → evdev (`tca8418_keypad_m5stack`) |
| Audio | ES8389 codec @ 0x10 (I2S), AW8737A amp (enable GPIO24), HP detect GPIO17 |
| Power | BQ27220 fuel gauge @ 0x55, power-fail GPIO6 (`powerfail_suo`), no suspend-to-RAM |
| IO expander | M5IOE1 @ 0x4F: LCD reset, backlight, kbd/hub reset, Grove/HAT 5 V, mic mode |
| Other I2C1 | BMI270 0x68, BMM150 (some SKUs), RX8130CE RTC 0x32 |
| Radios | CYW43439 Wi-Fi 2.4 GHz (SDIO, `brcmfmac`), BT over UART (`hciuart`) |
| USB | `dwc2` → GL852G hub → SR9900A Ethernet, USB-A, USB-C, HAT; hardware host/device switch |
| Camera | IMX219 CSI (non-Lite only) via `unicam` + libcamera |
| HDMI | VC4 KMS (`vc4-kms-v3d`) |

## 3. What the stock image does today (and where the waste is)

From `CardputerZero/pi-gen` branch `cardputerzero_v1.1` (commit `3b0e6f6`), `stage2/05-cardputerzero`:

- Built from **pi-gen stage 0–4** = full Raspberry Pi OS **desktop** (`rpd-wayland-core`,
  `rpd-x-core`, `rpd-applications`, `rpd-developer`, …) on a 512 MB device whose UI is an SPI
  LCD that doesn't use the desktop. `labwc` runs headless on a `NOOP-1` output.
- `gpu_mem=128` **and** `dtoverlay=vc4-kms-v3d,cma-64`.
- Keeps `build-essential` + `linux-headers-rpi-v8` + `device-tree-compiler` + `git` in the image
  ("for user driver builds"), plus `fonts-noto-cjk` and `fonts-noto-cjk-extra`.
- Stage 2 adds `cloud-init`, `avahi-daemon`, `rpi-connect-lite`, `udisks2`, `modemmanager`-less
  NetworkManager, `bluez`, `gdb`, `luajit`, `rpi-update`, `apt-listchanges`.
- PipeWire + WirePlumber + pipewire-pulse + rtkit (stock apps link `libpulse0`).
- Persistent journal on the SD card (`SystemMaxUse=50M`).
- Fixed 14.8 GiB image, no resize. Default user `pi` / `raspberry`, SSH **enabled**,
  timezone `Asia/Shanghai`, Wi-Fi regdomain hard-coded `cfg80211.ieee80211_regdom=AE` in cmdline.
- The M5 driver repo is cloned **unpinned from `main`** at build time, and the modules are built
  once into `/lib/modules/<kver>/extra` with **no DKMS**, so an `apt upgrade` of the kernel
  silently drops the LCD/keyboard/audio drivers.
- Launcher (`applaunch` 0.6.111): C/C++ LVGL 9.5, ~99 MB installed (35 MB binaries + 64 MB
  assets), runs as a **systemd user service with linger** for UID 1000.

**Measured** power costs (`InstantComet/cz-battery-saver`, `vasilpetrov/cardputerzero-batt-opt`):

| Cause | Cost |
|---|---|
| Stock idle, screen on | 416 mA (≈4.2 h on 1750 mAh) |
| USB Ethernet chip never runtime-suspends → hub + `dwc2` awake, **~8000 IRQ/s** | ~105 mA |
| Launcher keeps an audio stream open → ES8389 + I2S DMA never stop | 35–40 mA |
| Launcher keeps redrawing the LCD over SPI with backlight off | ~30 mA |
| Wi-Fi on (screen off, cumulative delta) | ~38 mA |
| PipeWire running (screen off, cumulative delta) | ~36 mA |
| Backlight | ~30 mA |
| Navigating the launcher UI | 500–700 mA |
| Floor (CM0 alone, everything off) | ~173 mA |

BT, CPU cap at 600 MHz, HDMI and the camera made no measurable power difference.

**Measured** RAM: a running HDMI desktop costs ~55 MB with labwc (`cz-desk-mode`) and 75–90 MB with
LightDM + LXDE (Kali image).

## 4. Memory plan (ordered by expected win; all to be measured)

1. **`gpu_mem` 128 → 64 (default).** `gpu_mem` is carved out before Linux boots, so the 64 MB
   returns to Linux directly (~12.5% of total RAM). The camera stays in the base image (IMX219 via
   `unicam` + libcamera, whose ISP runs on the VPU), and 64 MB is expected to cover the ISP and H.264
   codec (to verify). Check the custom-`start.elf` boot splash, camera preview/capture and H.264 at 64.
2. **No desktop in the base image.** No `rpd-*` metapackages, LightDM, labwc or Xorg. Desktop mode
   comes later as an installable launcher app (§6). **Measured:** a running desktop costs ~55 MB RAM
   and ~20 mA (`cz-desk-mode`); LXDE costs 75–90 MB (Kali image).
3. **Keep `vc4-kms-v3d,cma-64`.** Under KMS the GPU and camera pipeline draw from CMA, not
   `gpu_mem`, so `gpu_mem` can still be cut (item 1). HDMI needs vc4, and the future desktop app
   depends on it. The camera is in the base image, so CMA must fit full-res capture: IMX219
   3280×2464 raw is ≈10 MB per buffer, with several buffers in flight. Verify full-res stills and
   preview with `cma-64` in Phase 1. Idle CMA isn't wasted: movable pages (page cache, anon) borrow
   it until HDMI or the camera needs it.
4. **Never installed.** The build is additive (minbase), so these are a checklist against
   Recommends pulling them in, not a removal step: `cloud-init`, `avahi-daemon`,
   `rpi-connect-lite`, `udisks2`, `ModemManager`, `triggerhappy`, `cups`, `rpcbind`, LightDM,
   `piwiz`, `userconfig`. Timers off: `apt-daily*`, `man-db.timer`, `e2scrub*`, `fstrim.timer`
   (keep the service for manual use).
   **Bluetooth:** keep `hciuart`/`bluetooth` disabled until the user turns BT on. **Don't use
   `dtoverlay=disable-bt`:** it remaps `serial0` from the mini-UART (`/dev/ttyS0` on GPIO14/15,
   used by the GPS/LoRa Cap apps) to the PL011, which breaks them. It's also boot-time only, so a
   BT toggle would need a reboot.
5. **Audio on demand.** Keep PipeWire (stock apps need `libpulse0`). Debian already socket-activates
   it; what keeps it running on stock is `APPLaunch.service`'s `Wants=/After=pipewire-pulse.service`.
   The Rust launcher's unit must not `Want` PipeWire, and it connects only while it plays a sound.
   Leave WirePlumber's node suspend-on-idle on so the codec powers down. With key sounds off and no
   audio app running, PipeWire never starts.
6. **Wi-Fi stack.** Keep NetworkManager for app compatibility, but use `wifi.backend=iwd`
   instead of `wpa_supplicant`. Measure both; `iwd` + `systemd-networkd` alone is lighter still
   if no app needs NM's D-Bus API.
7. **zram.** Check what `rpi-swap` sets up first (`swapon --show`, `/etc/rpi/swap.conf*`). Target:
   zram only, `zstd`, ~50% of RAM, `vm.swappiness=100–150`, `vm.page-cluster=0`. No swap file on SD.
8. **Disk diet (not RAM, but image size and update time).** Drop `build-essential`, kernel headers,
   `gdb`, `luajit`, `rpi-update`, CJK `-extra` fonts and docs/man pages (dpkg `path-exclude`).
   Small image + first-boot root resize instead of a fixed 14.8 GiB image. **Risk to test:** M5
   deliberately removed first-boot resize (their notes were deleted in `3b0e6f6`), while Kali's
   `cz-firstboot-resize` works on this device.
9. **Kernel (phase 3).** Build our own `rpt` 6.18 kernel package with the M5 drivers in-tree and a
   trimmed config (no unused SoC/FS/net drivers). Until then, **pin** the kernel (`apt-mark hold`)
   and ship the M5 modules as our own `.deb` built in CI per kernel version, with the driver commit
   pinned.

## 5. Boot and performance

- **Boot:** no initramfs, no cloud-init, `quiet loglevel=3`, `fbcon=map:99` (keep the console off
  the LCD), `camera_auto_detect=0` with an explicit `dtoverlay=imx219` (as M5's `config_setup`
  does), `display_auto_detect=0`, `disable_splash=1` once our own splash works, `boot_delay=0`.
  Tune with `systemd-analyze critical-chain`.
- **USB autosuspend:** ship the udev rule setting the SR9900A to `power/control=auto` (from
  `cz-battery-saver`). Match it by USB VID:PID, not driver name (cz-battery-saver names `r8152`,
  so the driver isn't certain). This alone is −105 mA and removes the ~8000 IRQ/s storm, which
  also frees CPU.
- **CPU:** governor `schedutil`, keep stock 1.0 GHz (no overclock in a sealed handheld).
- **Storage:** ext4 `noatime,commit=60`, no persistent journal by default (§7). `/tmp` on tmpfs
  **capped at `size=32M`**: the tmpfs default is 50% of RAM, and ffmpeg/camera/apt temp files land
  there. Big temp files belong in `/var/tmp` (on disk).
- **Display throughput:** a full 320×170 RGB565 frame is 108,800 bytes ≈ 15–21 ms over SPI
  (≤60 MHz requested; the real clock depends on the core-clock divider). Full redraws therefore cap
  around 45–65 fps, so partial (damage) updates are the main lever. The TE pin is unused in the DT
  (possible tearing on fast animation).

## 6. Desktop mode: later, as an app

Not part of the base image. On the stock image the HDMI desktop can't be driven from the built-in
keyboard, which sits on its own seat (`ID_SEAT`) so the desktop never sees it. Desktop mode will be
built later as an installable launcher app that fixes this. In the base OS, plugging in HDMI starts
nothing.

**What the base OS keeps so the app can be added later:**

- `vc4-kms-v3d` + CMA (§4 item 3).
- UID 1000 in the `video` and `input` groups.
- The LCD and built-in keyboard stay off the default seat.

**Notes for the future app** (prior art: `InstantComet/cz-desk-mode`, which solves the keyboard
problem on this device):

- **Keyboard:** `EVIOCGRAB` the TCA8418 and replay keys through `/dev/uinput` on the default seat,
  translating the Sym-layer codes to standard key + Shift. Give it back on `Ctrl+Alt+Esc`, HDMI
  unplug or exit. While the keyboard is handed over, the LCD is off and the launcher doesn't render.
  The app's package adds a udev rule letting `input` use `/dev/uinput`.
- **Compositor:** `labwc` as a UID 1000 user unit with `LIBSEAT_BACKEND=noop` on the vc4 card only.
  It shares the launcher's PipeWire/D-Bus session. **Never LightDM:** its greeter's PipeWire takes
  the sound card and deadlocks the launcher's audio.
- **Packages:** ship the desktop with the app, not the image. Leave out PackageKit, the panel's
  updater widget, `rpi-connect` and `wayvnc` autostart (privacy, §7). Keep the browser optional
  because of RAM.
- **Polkit:** no logind session, so polkit rules for UID 1000 are needed for NetworkManager/BlueZ
  actions from the desktop.
- **Cost:** ~55 MB RAM and ~20 mA while running, so stop it on HDMI unplug after a debounce.

## 7. Privacy: no telemetry, logging off by default

**What was audited:** every URL string in every executable of the `applaunch` 0.6.111 deb
(`bin/`, `libexec/`), cross-checked against its public source (`CardputerZero/launcher`), plus the
image build scripts. **Not audited:** the other stock apps (Music, Files, Camera, Recorder, Cap
apps, …) and runtime traffic. The acceptance test for "no phone-home" is the §9 idle `tcpdump`.

Findings in the launcher package:

- No analytics/crash-reporting endpoints.
- The launcher/app update check (`api.github.com/repos/CardputerZero/*`) runs **only when the user
  presses Enter** on Settings → Update.
- **ZClaw** (bundled AI chat app) probes `https://www.cloudflare.com/cdn-cgi/trace` and
  `https://www.msftconnecttest.com/connecttest.txt` **automatically on startup**. Cloudflare's trace
  returns the caller's IP and location. It also has provider endpoints (OpenAI, Anthropic,
  OpenRouter, DeepSeek), which are used only once a user configures them. Action: don't ship ZClaw,
  or patch the probe out.
- **AppStore** fetches `cardputerzero.github.io/generated/registry.json` and
  `cardputer-zero-repo.oss-cn-shenzhen.aliyuncs.com/packages/cn/registry.json`. It isn't known yet
  whether this happens on open or on refresh. Action: our launcher fetches catalogs only when the
  user asks, from a mirror we choose.
- M5's imager fork ships with `TELEMETRY_URL ""` (RPi Imager's download stats removed).

The bigger leaks are in the base OS and its defaults:

| Source in stock image | What it does | Action |
|---|---|---|
| `rpi-connect-lite` | Raspberry Pi Connect cloud remote-access client | Don't install |
| `cloud-init` (+ `rpi-cloud-init-mods`) | Network/metadata datasources at boot | Don't install; first-boot setup done by the launcher |
| `avahi-daemon` | Broadcasts hostname/services via mDNS on every network | Don't install (optional package) |
| `apt-daily*`, `apt-listchanges`, `applaunch-apt-update` / `applaunch-updater` / `applaunch-apps-updater` | Contact Debian/RPi/GitHub servers | No timers; updates only when the user starts them |
| Stock launcher update check (`api.github.com/repos/CardputerZero/…`) | Version check against GitHub (already user-initiated) | Rust launcher: keep it manual only |
| ZClaw startup probe, AppStore catalogs | See findings above | Drop/patch ZClaw; catalogs fetched only on user request |
| Persistent journal (50 MB on SD) | Keeps logs across boots | `Storage=none` by default (see below) |
| NetworkManager DHCP | Sends hostname `cardputerzero` (identifies the device type) | `dhcp-send-hostname=false`, neutral hostname, `wifi.cloned-mac-address=stable-ssid` (or `random`), scan MAC randomization on |
| NetworkManager connectivity check | Pings a check URL if configured | `[connectivity] enabled=false` |
| `systemd-timesyncd` | NTP to the Debian pool (reveals IP) | Keep for TLS (RTC is present), make the server configurable; optional chrony + NTS |
| SSH on, `pi`/`raspberry` | Security, not telemetry | SSH off by default; password set at first boot |
| `Asia/Shanghai` TZ, `regdom=AE` cmdline | Wrong defaults | UTC default; regdomain chosen at first boot |
| Bluetooth | Discoverable name | Off by default (`rfkill default_state=0` blocks **all** radios at boot, Wi-Fi included; keep it, and the launcher unblocks on demand, §8) |
| ADB gadget | Serial derived from `machine-id` hash | Already off by default; keep off |
| `systemd-coredump`, `popularity-contest`, `reportbug` | Crash/usage reporting | Not installed (minbase); `kernel.core_pattern=|/bin/false` |
| Kernel log | Readable by any user | Set `kernel.dmesg_restrict=1` and `kernel.kptr_restrict=2` explicitly (Debian kernels default to restrict; the rpt config is unverified) |
| M5 Imager (host tool) | RPi Imager telemetry stripped in the fork. OS customisation uses `init_format: "systemd"` (a `firstrun.sh` on the FAT partition calling `raspberrypi-sys-mods` helpers), **not cloud-init** | **Decided: no Imager customisation; all setup happens in the launcher.** Don't ship `raspberrypi-sys-mods`. Our os-list entry uses `init_format: "none"`. We publish `.img.xz` + `bmap`; flash with `bmaptool`/`dd`/any imager, and flashing docs say to skip OS customisation |

**Logging design (default off, toggle in the launcher):**

- Default `/etc/systemd/journald.conf.d/10-cpzero-privacy.conf`: `Storage=none`,
  `ForwardToSyslog=no`, `ForwardToKMsg=no`, `ForwardToConsole=no`.
- Launcher setting **Logs: Off / RAM only / Persistent** writes `50-cpzero-logging.conf`
  (`Storage=none|volatile|persistent`, with `RuntimeMaxUse=8M` / `SystemMaxUse=16M`) and restarts
  `systemd-journald`. The privileged step goes through a fixed root helper unit allowed by a polkit
  rule, the same pattern as the stock `60-applaunch-updater.rules`. Switching back to Off also
  vacuums the existing journal.
- The launcher's own logs go to stderr → journald, so they follow the same switch.
- No rsyslog, no `/var/log` text logs; `apt`/`dpkg` logs can stay (local only).

## 8. Rust launcher: OS contract and design rules

**Contract to keep so stock apps and the AppStore keep working:**

- Discover apps from `/usr/share/APPLaunch/applications/*.desktop`. Keys: `Name`, `Exec`, `Icon`
  (relative to `/usr/share/APPLaunch/` or absolute), `Terminal`, `Sysplause`. Reject `Exec` with
  shell metacharacters. Use inotify instead of the stock 3 s polling.
- `Terminal=true` means the launcher needs a **built-in PTY terminal page**.
- Launch with fork+exec in a new process group. While the child runs, don't draw and ignore keys
  (the evdev device is **shared, not grabbed**). ESC-hold → `SIGTERM` then `SIGKILL` to the pgid.
- Apps draw to `/dev/fb_lcd` (fbdev emulation). Before exec the launcher must **close its DRM fd,
  or render via fbdev itself**, and reopen after the child exits. Whether a plain DRM-master drop is
  enough for fbdev output to show is **to verify**. `fb_load` hands off by writing the logo to fbdev
  and closing its DRM fd.
- Runs as UID 1000 in a lingering user session (PipeWire, session D-Bus, `XDG_RUNTIME_DIR`), with
  `WorkingDirectory=/usr/share/APPLaunch`.
- **Gotcha:** `fb_load` (boot splash) hard-codes `/usr/share/APPLaunch/bin/M5CardputerZero-APPLaunch`
  and `LaunchWizard` and keeps the DRM device open until one of them runs. With the Rust launcher at
  another path it never exits and likely blocks the launcher's display (to verify). Patch its watch
  list, or let the launcher own the splash.
- **Radios:** `rfkill default_state=0` blocks Wi-Fi and BT at every boot. The launcher unblocks
  them when the user enables them; `systemd-rfkill` persists that state across boots.
- **Hardware controls** via sysfs:
  - backlight in `/sys/class/backlight` (stock masks `systemd-backlight`, so the launcher restores
    brightness itself);
  - M5IOE1 switches in `/sys/class/leds/{ext_5v_out,grove_5v_out,grove_fun,ext_usb_gpio_fun}`,
    with access granted by M5's `cardputerzero-leds.conf` / `60-cardputerzero-leds.rules`;
  - low-battery shutdown (cz-battery-saver defaults to 8%).

**Design rules taken from the measurements:**

- **Render only on change.** DRM dumb buffer in RGB565 + `DRM_IOCTL_MODE_DIRTYFB` with damage
  rects. The `mipi_dbi` pipeline merges damage, so only dirty rows go over SPI. No animation loop
  while idle, and nothing rendered at all while the backlight is off or a child app runs.
- **Audio only while playing.** Open the stream per sound, close it after, and let the sink suspend
  (−35–40 mA).
- **Single-threaded event loop** (epoll: evdev, timerfd, signalfd, inotify, udev netlink for
  `power_supply`/hotplug). No multi-threaded tokio runtime; `zbus` for NM/BlueZ calls.
- **Battery** from `/sys/class/power_supply/` on uevent or a 30 s timer, not tight polling.
- **Build:** `opt-level="s"`, `lto="fat"`, `codegen-units=1`, `panic="abort"`, `strip=true`.
  Target RSS and binary size are to be measured against the stock launcher.
- **UI toolkit:** Slint's software renderer (partial rendering, RGB565, own platform backend;
  check its GPLv3 / royalty-free / commercial license fits) or `embedded-graphics` with our own
  widgets for the smallest footprint. Avoid `egui`/`iced`: they redraw continuously or need a GPU.
- Owns the privacy settings: logs (§7), updates (manual), Wi-Fi MAC policy, SSH/ADB/BT toggles,
  first-boot setup **on the device** (keyboard tutorial, user + password, Wi-Fi, timezone, regdomain,
  SSH/ADB opt-in). The only setup path: there is no Imager customisation or cloud-init. Until setup
  completes, SSH and ADB stay off, and the only network connection is the Wi-Fi the user joins in setup.

## 9. Baseline to measure on a device (before changing anything)

```sh
free -m; grep -E 'MemTotal|MemAvailable|Cma' /proc/meminfo; vcgencmd get_mem gpu
ps -eo rss,comm --sort=-rss | head -25          # or: smem -tk
systemctl list-units --type=service --state=running
systemd-analyze; systemd-analyze critical-chain
swapon --show; cat /proc/interrupts | grep dwc2   # sample twice, 10 s apart
cat /sys/class/power_supply/*/current_now        # idle, screen on / off
ss -tupan                                        # listening + outbound sockets
```

`smem` and `tcpdump` aren't on the stock image. To avoid changing the system being measured, capture
the 10-minute idle traffic on the Wi-Fi AP/router (or a laptop hotspot) and list every host the
device contacts.

Repeat the same set on each build to track the effect of every change.

## 10. Roadmap

1. **Phase 0:** baseline the stock image (§9).
2. **Phase 1:** mmdebstrap trixie minbase image + pinned kernel + M5 modules `.deb` (pinned commit)
   + privacy defaults + USB autosuspend rule, running the **stock** APPLaunch for parity testing.
   - Targets hardware rev v0.5, like M5's CI (`CONFIG_CARDPUTERO_V0_5`: `cardputerzero-v5-overlay`,
     `bq27220_v5`, `bmi270_bmm150_overlay`). Verify the same image boots cleanly on the Lite SKU (no
     camera/IMU, so the imx219/bmi270 probes should fail harmlessly).
   - Disable the bundled **ZClaw** (startup probes to Cloudflare and Microsoft) in
     `/usr/share/APPLaunch/preinstalled-desktop-apps.tsv`.
   - Still ship what stock APPLaunch uses at runtime: NetworkManager, BlueZ, PipeWire-pulse,
     polkit + `60-applaunch-updater.rules`, and `fb_load`.
3. **Phase 2:** Rust launcher replaces APPLaunch (same `.desktop` contract), with logging and
   privacy settings. Patch or replace `fb_load`.
4. **Phase 2b:** desktop mode as an installable launcher app (§6).
5. **Phase 3:** custom kernel with M5 drivers in-tree and trimmed config; tune CMA size. Optional
   Buildroot appliance build.

## 11. Open questions

1. Must stock M5 apps and the AppStore keep working? This decides Debian vs Buildroot.
2. Do we have a device to baseline on?

## Sources

- M5Stack docs: https://docs.m5stack.com/en/CardputerZero ,
  https://docs.m5stack.com/en/compute/cardputerzero/image
- CNX Software: https://www.cnx-software.com/2026/05/25/cardputerzero-a-raspberry-pi-cm0-pocket-computer-for-makers/
- Official image build: https://github.com/CardputerZero/pi-gen (branch `cardputerzero_v1.1`)
- Drivers and overlays: https://github.com/m5stack/m5stack-linux-dtoverlays (`modules/CardputerZero`, `st7789v-1.0`)
- Stock launcher (source and release debs): https://github.com/CardputerZero/launcher
- Kali image (mmdebstrap precedent): https://github.com/on-my-machine-works/kali-on-cardputerzero
- Power measurements: https://github.com/InstantComet/cz-battery-saver , https://github.com/vasilpetrov/cardputerzero-batt-opt
- On-demand HDMI desktop (prior art): https://github.com/InstantComet/cz-desk-mode
- Bring-up notes (kernel/OS version): https://github.com/oumike/camillia-mt/issues/155
- Repo index: https://github.com/UnAneOnyme/CardputerZeroCollection
- Imager fork: https://github.com/CardputerZero/m5stack-imager
