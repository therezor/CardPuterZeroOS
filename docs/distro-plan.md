# Gh0st OS: base distro and optimization plan

The research behind the image. Figures marked **measured** come from community repos (linked);
everything else is a target to measure. `image-build.md` has how the image is built and where
it differs from this plan. Debian supplies everything except the Raspberry Pi
packages `os/lock.json` pins; the kernel boots without an initramfs; stock apps need
`libcamera0.7`, not 0.8.

## 1. Recommendation

**Base: Debian 13 "trixie" arm64, built additively with `mmdebstrap --variant=minbase`,
plus the Raspberry Pi `rpt` 6.18 kernel/firmware and M5Stack's CardputerZero drivers,
with the kernel pinned.** Camera included, `gpu_mem=64`. No desktop in the image. No cloud-init; logging and network services are off by default.

Why not another distro:

- **Drivers:** the LCD, keyboard, audio codec, IO expander and fuel gauge are all out-of-tree
  M5 modules (`m5stack/m5stack-linux-dtoverlays`), proven only against the RPi `6.18.x+rpt-rpi-v8`
  kernel. Any distro must carry that kernel + those modules.
- **Apps:** the whole CardputerZero app ecosystem (M5 apt repo, AppStore, GitHub releases) is
  **arm64 glibc `.deb`s built for trixie** that depend on `libpulse0`, `libcamera0.7`,
  `ffmpeg`, etc. Buildroot/Alpine (musl) break all of them.
- **Precedent:** `on-my-machine-works/kali-on-cardputerzero` already builds a working image this
  way (mmdebstrap minbase + RPi kernel/firmware + M5 modules), so the path is proven.

Buildroot stays an option for a later "appliance" variant **only if** we drop stock-app/AppStore
compatibility. It would have the lowest RAM use and fastest boot, but it needs
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
2. **No desktop in the base image.** No `rpd-*` metapackages, LightDM, labwc or Xorg. **Measured:** a running desktop costs ~55 MB RAM
   and ~20 mA (`cz-desk-mode`); LXDE costs 75–90 MB (Kali image).
3. **Keep `vc4-kms-v3d,cma-64`.** Under KMS the GPU and camera pipeline draw from CMA, not
   `gpu_mem`, so `gpu_mem` can still be cut (item 1). HDMI needs vc4, and the future desktop app
   depends on it. The camera is in the base image, so CMA must fit full-res capture: IMX219
   3280×2464 raw is ≈10 MB per buffer, with several buffers in flight. Full-res stills and
   preview fit in `cma-64` with three raw buffers (`hardware-facts.md`, Camera). Idle CMA isn't wasted: movable pages (page cache, anon) borrow
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
   The shell's unit doesn't `Want` PipeWire, but the shell calls `wpctl` at start (the saved
   volume), and PipeWire and WirePlumber run from boot. WirePlumber suspends a node with no
   stream: at idle the sink is suspended and the codec closed with PipeWire running
   (`hardware-facts.md`). The shell opens no stream, and it parks a paused app's streams
   (`apps.md`, Pausing).
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
9. **Kernel.** No kernel of our own. The `rpt` 6.18 kernel is **pinned** in `os/lock.json`, and the
   M5 modules ship as our own `.deb` (`gh0st-panel-modules`) built per kernel version, with the
   driver commit pinned.

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
- **Storage:** ext4 `noatime,commit=60`, no persistent journal by default (§6). `/tmp` on tmpfs
  **capped at `size=32M`**: the tmpfs default is 50% of RAM, and ffmpeg/camera/apt temp files land
  there. Big temp files belong in `/var/tmp` (on disk).
- **Display throughput (measured, `hardware-facts.md`):** SPI runs at 50 MHz (400 MHz VPU / 8). A
  full 108,800-byte frame takes 17.6 ms, so partial updates are the main lever. Upstream fbdev waits
  50 ms before each send. gh0st's patched driver cuts that to one jiffy (4 ms) for every fbdev app,
  uses the TE pin and runs the panel at 50 Hz. gh0st also calls `fsync()` after each flush.

## 6. Privacy: no telemetry, logging off by default

**What was audited:** every URL string in every executable of the `applaunch` 0.6.111 deb
(`bin/`, `libexec/`), cross-checked against its public source (`CardputerZero/launcher`), plus the
image build scripts. **Not audited:** the other stock apps (Music, Files, Camera, Recorder, Cap
apps, …) and runtime traffic. The acceptance test for "no phone-home" is the §8 idle `tcpdump`.

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
  `cardputer-zero-repo.oss-cn-shenzhen.aliyuncs.com/packages/cn/registry.json`, on every open,
  and all 179 icons with it (measured, `hardware-facts.md`). Our Store
  (`store.md`) reads a signed copy on our Pages, only when the user refreshes or with the daily
  check, and sends no account or device ID.
- M5's imager fork ships with `TELEMETRY_URL ""` (RPi Imager's download stats removed).

The bigger leaks are in the base OS and its defaults:

| Source in stock image | What it does | Action |
|---|---|---|
| `rpi-connect-lite` | Raspberry Pi Connect cloud remote-access client | Don't install |
| `cloud-init` (+ `rpi-cloud-init-mods`) | Network/metadata datasources at boot | Don't install; first-boot setup done by the launcher |
| `avahi-daemon` | Broadcasts hostname/services via mDNS on every network | Don't install (optional package) |
| `apt-daily*`, `apt-listchanges`, `applaunch-apt-update` / `applaunch-updater` / `applaunch-apps-updater` | Contact Debian/RPi/GitHub servers | Our daily check instead (`updates.md`): one timer, off in Settings, installs only when the user confirms |
| Stock launcher update check (`api.github.com/repos/CardputerZero/…`) | Version check against GitHub (already user-initiated) | Rust launcher: keep it manual only |
| ZClaw startup probe, AppStore catalogs | See findings above | Drop/patch ZClaw. Catalogs: done, our Store fetches ours on request or daily |
| Persistent journal (50 MB on SD) | Keeps logs across boots | `Storage=none` by default (see below) |
| NetworkManager DHCP | Sends hostname `cardputerzero` (identifies the device type) | `dhcp-send-hostname=false`, neutral hostname, `wifi.cloned-mac-address=stable-ssid` (or `random`), scan MAC randomization on |
| NetworkManager connectivity check | Pings a check URL if configured | `[connectivity] enabled=false` |
| `systemd-timesyncd` | NTP to the Debian pool (reveals IP) | Keep for TLS (RTC is present), make the server configurable; optional chrony + NTS |
| SSH on, `pi`/`raspberry` | Security, not telemetry | SSH off by default; password set at first boot |
| `Asia/Shanghai` TZ, `regdom=AE` cmdline | Wrong defaults | UTC default; regdomain chosen at first boot |
| Bluetooth | Discoverable name | Off by default (`rfkill default_state=0` blocks **all** radios at boot, Wi-Fi included; keep it, and the launcher unblocks on demand, §7) |
| ADB gadget | Serial derived from `machine-id` hash | Already off by default; keep off |
| `systemd-coredump`, `popularity-contest`, `reportbug` | Crash/usage reporting | Not installed (minbase); `kernel.core_pattern=|/bin/false` |
| Kernel log | Readable by any user | Set `kernel.dmesg_restrict=1` and `kernel.kptr_restrict=2` explicitly (Debian kernels default to restrict; the rpt config is unverified) |
| M5 Imager (host tool) | RPi Imager telemetry stripped in the fork. OS customisation uses `init_format: "systemd"` (a `firstrun.sh` on the FAT partition calling `raspberrypi-sys-mods` helpers), **not cloud-init** | **No Imager customisation; all setup happens on the device (`setup.md`).** Don't ship `raspberrypi-sys-mods`. Our os-list entry uses `init_format: "none"`. We publish `.img.xz` + `bmap`; flash with `bmaptool`/`dd`/any imager, and flashing docs say to skip OS customisation |

**Logging design (default off, toggle in the launcher):**

- Default `/usr/lib/systemd/journald.conf.d/50-gh0st.conf`: `Storage=none`,
  `ForwardToSyslog=no`, `ForwardToKMsg=no`, `ForwardToConsole=no`.
- Launcher setting **Logs: Off / RAM only / Persistent** writes `/etc/systemd/journald.conf.d/zz-gh0st-logging.conf`
  (`Storage=none|volatile|persistent`, with `RuntimeMaxUse=8M` / `SystemMaxUse=16M`) and restarts
  `systemd-journald`. The privileged step goes through a fixed root helper unit allowed by a polkit
  rule, the same pattern as the stock `60-applaunch-updater.rules`. Switching back to Off also
  vacuums the existing journal.
- The launcher's own logs go to stderr → journald, so they follow the same switch.
- No rsyslog, no `/var/log` text logs; `apt`/`dpkg` logs can stay (local only).

## 7. Rust launcher: OS contract and design rules

**Contract to keep so stock apps and the AppStore keep working:**

- Discover apps from `/usr/share/APPLaunch/applications/*.desktop`. Keys: `Name`, `Exec`, `Icon`
  (relative to `/usr/share/APPLaunch/` or absolute), `Terminal`, `Sysplause`. Reject `Exec` with
  shell metacharacters. Use inotify instead of the stock 3 s polling.
- `Terminal=true` runs the entry in the terminal app (`apps.md`).
- Launch with fork+exec in a new process group. While the child runs, the shell draws only what
  the app asks for (`apps.md`) and reads only ESC and the global keys (the evdev device is
  **shared, not grabbed**). ESC-hold → `SIGTERM` then `SIGKILL` to the pgid.
- Apps draw to `/dev/fb_lcd` (fbdev emulation). The shell draws through fbdev too, so there is
  no DRM master to hand over.
- Runs as UID 1000 in a lingering user session (PipeWire, session D-Bus, `XDG_RUNTIME_DIR`), with
  `WorkingDirectory=/usr/share/APPLaunch`.
- `fb_load` (boot splash) hard-codes the stock launcher and `LaunchWizard` paths and holds the
  panel until one of them runs. `packaging/systemd/fb_load.service.d` drops `--keep-open`, so it
  draws the logo and lets go.
- **Radios:** `rfkill default_state=0` blocks Wi-Fi and BT at every boot. The launcher unblocks
  them when the user enables them; `systemd-rfkill` persists that state across boots.
- **Hardware controls** via sysfs:
  - backlight in `/sys/class/backlight` (stock masks `systemd-backlight`, so the launcher restores
    brightness itself);
  - M5IOE1 switches in `/sys/class/leds/{ext_5v_out,grove_5v_out,grove_fun,ext_usb_gpio_fun}`,
    with access granted by M5's `cardputerzero-leds.conf` / `60-cardputerzero-leds.rules`;
  - low-battery shutdown (cz-battery-saver defaults to 8%).

**Design rules.** The shell's rules live in `CLAUDE.md` and the code: one
`poll` loop with no threads, fbdev with row-level flushes. What it gives apps
is in `apps.md`. First-boot setup and the privacy settings are in `setup.md`.

## 8. Baseline to measure on a device (before changing anything)

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
