# Building the Gh0st OS image

The image is Debian 13 trixie arm64, built from pinned inputs in a container.
The reasons behind each choice are in `distro-plan.md`; the milestones in
`roadmap.md`. Everything here runs the same on the Mac and on CI.

## Quick start

Needs Docker Desktop running and `jq` on the Mac.

```sh
os/build.sh fetch           # download and verify every pinned input
os/build.sh debs            # build our packages and the drivers
os/build.sh image release   # os/out/gh0st-release-YYYYMMDD.img.xz
os/build.sh check release   # tier 1 checks
os/build.sh smoke           # first boot of the image in QEMU, about 10 min
```

A clean build takes about ten minutes. `.github/workflows/os.yml` runs the
same steps on an arm64 runner; it has not run yet. Work happens on the Docker volume
`gh0st-work`, never on the repo: macOS filesystems lose ownership and
sparse files. Only finished files land in `os/out/`.

## First-boot setup

A release ships with no user. `/var/lib/gh0st/setup` is the marker:
while it exists, `gh0st-setup.service` (in `gh0st`) runs `gh0st-settings
--setup` as root, and the shell's `gh0st.service` stays off. Setup makes UID
1000, removes the marker and starts the user's session (`docs/setup.md`).
`/var/lib/gh0st/setup-home` is setup's own `HOME`. A cable connects on
its own; Wi-Fi waits for setup's network step.

There is one image, the one users get. No dev image: a device under test
goes through setup like any other. To get in afterwards, turn on SSH, and
Logs if needed, in Settings.

## Flash

```sh
diskutil list                                   # find the card: /dev/diskN
scripts/flash.sh os/out/gh0st-release-YYYYMMDD.img.xz /dev/diskN
```

The script refuses internal disks and asks before it writes.

## Layout

```
os/build.sh            host wrapper: builds the container, runs a step
os/Dockerfile          the build container
os/lock.json           every pinned input: URL, SHA-256, commit, snapshot
os/image/build-image.sh  the steps, inside the container
os/image/manifest/     roots.list, deny.list, exec.list, boot-files.list
os/image/variants/     release: extra roots, deny list, the lock,
                       expected units, budget
os/image/hooks/        the mmdebstrap customize hook
os/image/checks/       tier 1 checks
os/packages/           our packages, built by build-deb.sh
```

`packaging/` stays where it is: `scripts/deploy.sh` and the `gh0st` package
both install from it.

## How a build runs

1. **fetch.** Every deb in `lock.json` is downloaded and checked by SHA-256:
   the Raspberry Pi kernel, firmware and libcamera, the stock apps, and the
   applaunch deb the AppStore is repacked from. The M5 driver repo is fetched
   at its pinned commit. `cargo fetch` runs here, so later steps are offline.
2. **debs.** The pinned debs become a local repo. Our packages build:
   - `cargo build` for gh0st, gh0st-settings, gh0st-term, gh0st-update and
     gh0st-camera.
   - The camera helper (C++), against `libcamera-dev` unpacked into a
     sysroot. That deb's `use` is `camera-build`, so it is fetched but never
     installed. `os/build.sh camd` builds the helper alone into
     `os/out/camd`. The `gh0st-camera` deb is built but not in
     `roots.list` until it replaces M5's Camera (`docs/camera.md`).
   - The M5 drivers and our panel driver, against the pinned kernel headers.
     M5's `make install` writes to absolute paths, so only `make all` runs
     and an explicit file list goes into the package. Every module's
     vermagic must match the pinned kernel.
3. **image.** `mmdebstrap --variant=minbase` installs `roots.list` from a
   snapshot.debian.org timestamp and the two local repos. Recommends are
   off, and docs, man pages and translations never unpack. The customize
   hook leaves setup's marker where a user would be, runs `systemctl
   preset-all` and removes per-device identity. The boot and home trees come out as
   their own trees.
4. The FAT partition is made with `mkfs.vfat` and `mcopy`, the ext4 ones with
   `mke2fs -d` from the tar, and the disk with `sfdisk` on a sparse file. No
   loop devices.
5. `e2fsck`, `fsck.vfat`, `bmaptool create`, `xz`, SHA-256.

The container needs `CAP_SYS_ADMIN` and no seccomp or AppArmor mount filter:
mmdebstrap mounts `/proc`, `/sys` and `/dev` in its chroot. Without them it
still builds, but maintainer scripts run without `/proc`.

## What is in the image

**Debian first.** Debian supplies libc, OpenSSL, NetworkManager,
wpa_supplicant, BlueZ, PipeWire and ffmpeg. Raspberry Pi builds come only
where `lock.json` pins them: the kernel, `raspi-firmware`, the Wi-Fi and
Bluetooth firmware, libcamera and its IPA, libpisp, TensorFlow Lite (the
IPA needs it) and `vcgencmd`. The device itself has no Raspberry Pi apt
source. Old libcamera and kernel builds are fetched from the pool by URL:
the live index has moved on (libcamera-ipa 0.8, kernel 6.18.60), so a build
from it would not resolve.

**Base apps.** gh0st, Settings, the terminal, and the stock Camera, Files,
Music and Recorder at the versions the dev device runs. M5's AppStore is
repacked alone (`gh0st-m5-appstore`): the binary, its images, three fonts
and two sounds. Not the launcher and not `applaunch-updater`, which would
reinstall the stock launcher. It installs apps through `sudo`. It fetches
catalogs from GitHub and an Aliyun mirror and asks ipinfo.io for the
country; that stays until our own Store replaces it.

**Command-line tools.** What a Linux user reaches for in the terminal:
curl, wget, python3, htop, less, nano, vi (`vim-tiny`), file, lsof, zip,
ping, traceroute, nc, killall, lsusb, jq, tree and Tab completion. For the
pins and the Grove port: i2c-tools, gpiod, spi-tools and picocom, and
their Python modules (`gpiod`, `smbus2`, `spidev`, `serial`). The user is in
the `gpio`, `i2c`, `spi` and `dialout` groups, so none needs sudo, and
`/etc/profile.d/gh0st-sbin.sh` puts the sbin folders, where `i2cdetect`
lives, on the user's PATH. None runs a service. A device flashed before
them gets them once, from the migration `0001-cli-tools`
(docs/updates.md). `man-db` stays denied: its daily index job and groff
cost more than the pages are worth here.

**ffmpeg.** Files runs `ffmpeg` and `ffprobe` for video previews. Debian's
ffmpeg is a desktop build: its device library pulls OpenGL, so Mesa and
LLVM, and its filters pull speech synthesis and recognition. That is 148
packages and 367 MB of the 842 MB installed. Kept by decision (2026-11-21).

**Packages.**

| Package | Holds |
|---|---|
| `gh0st-base` | Config only: apt sources and deny pins, dpkg excludes, unit presets, journald off, zram, sysctl, rfkill default, `/tmp` cap, the first-boot grow, udev rules in place of `raspberrypi-sys-mods`, NetworkManager privacy defaults, PipeWire realtime limits without rtkit, the default volume, the one-time migrations |
| `gh0st-kernel` | Exact-version Depends on the kernel, drivers and firmware, so they move together |
| `gh0st-no-initramfs` | Provides `linux-initramfs-tool`: the kernel has ext4 and the SD driver built in |
| `gh0st-panel-modules` | The 15 M5 drivers and our panel driver, overlays, udev rules, firmware files, keymap, `fb_load` |
| `gh0st`, `gh0st-term` | Our binaries, `.desktop` files, polkit files, the `gh0st.service` user unit, `gh0st-setup.service` |
| `gh0st-m5-appstore` | See above |
| `gh0st-wifi` | The built-in Wi-Fi chip's boot retry (a udev rule binds it again when its firmware load fails), and firmware for USB adapters with monitor mode, from Raspberry Pi's `firmware-atheros`, `firmware-mediatek` and `firmware-realtek` |

**Services.** System: NetworkManager and its on-demand dispatcher,
timesyncd, bluetoothd, `fb_load` (a oneshot, so setup draws after the logo),
the first-boot grow and first-boot setup while its marker exists. sshd
starts only once the user turns SSH on. User: gh0st, and
PipeWire's sockets. One timer, the daily update check
(`gh0st-update-daily.timer`, off from Settings); the tmpfiles timer is
masked, `/tmp` is a tmpfs. `units-*.expected` in each variant is the list the checks hold it to.

## Kernel and boot

- No initramfs. Root is found by PARTUUID.
- `raspi-firmware` and the kernel hooks fill `/boot/firmware` in the chroot.
  The boot partition then keeps only what `boot-files.list` names: the
  CM0's firmware, kernel and dtb, and all overlays. Our four overlays are
  copied in by `gh0st-panel-modules`.
- Stock Raspberry Pi firmware, not M5's custom `start.elf` with its LCD
  splash. `fb_load` draws the logo once the kernel is up.
- `config.txt` is the stock one minus the Pi 4/5 sections, `auto_initramfs`
  and the splash. `gpu_mem=64` stays first, where Settings edits it.

## Partitions and first boot

| Part | Type | Size | Mount |
|---|---|---|---|
| p1 | FAT32 | 256 MiB | `/boot/firmware` |
| p2 | ext4 | 3 GiB | `/` |
| p3 | ext4 | 256 MiB, grows to the end of the card | `/home` |

All mount `noatime,commit=60`. The disk ID is fixed (`0x67683073`, "gh0s"),
so every card shares PARTUUIDs; randomizing it at first boot is still to do.
On first boot `gh0st-grow-data.service` grows p3, and `x-systemd.growfs`
grows its filesystem online. The grow runs before `local-fs-pre.target`
and waits for udev: the new size makes the card's device units flap for a
second or two, and a check or mount of `/home` in that window is cut off.
`gh0st-setup.service` is also wanted by `home.mount`, so a late mount still
starts setup. `e2fsprogs` is in the image so systemd checks `/` and `/home`
at boot and replays their journals. p3 has 4 KiB blocks and descriptors reserved
for 1 TiB, not the 1 KiB blocks `mke2fs` picks at 256 MiB. A fake card, one
that claims more than it holds, corrupts whatever lands past its real size
(`docs/hardware-facts.md`). machine-id and the random seed are made by systemd.
SSH host keys are made the first time SSH starts, by an `ExecStartPre`
drop-in: Debian's `sshd-keygen.service` only runs on systemd's first boot.

## Checks

`os/build.sh check VARIANT` runs offline on the last image:

- The packages equal `packages.lock`. Our own packages' versions are masked.
- Nothing from the deny lists. Every `+rpt` package is pinned in `lock.json`.
- apt's manual packages are `roots.list` plus Debian's required base.
- Enabled units equal `units-*.expected`. No timer but the daily update check.
- No SSH host keys, empty machine-id, no random seed, no apt lists, no
  Wi-Fi profiles.
- Every overlay `config.txt` names is on the boot partition, and nothing for
  other boards.
- Setup's marker, no UID 1000, root locked.
- setuid bits survive `mke2fs -d`; the helper is a hard link.
- Every command in `exec.list` resolves. Our binaries are static, so no
  package metadata says what they run.
- Stock app and AppStore libraries resolve (`ldd`).
- Driver vermagic matches the kernel; depmod finds no unknown symbols.
- Package count and installed size stay within `budget.conf`.

## Smoke test

`os/build.sh smoke` boots the image in QEMU's `raspi3b`, a Pi 3 close to
the CM0, for ten minutes. QEMU has no panel, keyboard, Wi-Fi or VideoCore,
so it proves the boot path only. Its serial console stays silent with this
device tree, so the test reads the journal off the card file afterwards.
The image keeps no logs, so the test's own copy of the card gets a journald
drop-in for a persistent journal first. The card is 8 GB, so p3 grows; a
64 GB card stalls QEMU's slow SD emulation behind ext4's inode table
writes, which a real card finishes in seconds. It checks: root mounted without an
initramfs, p3 and its filesystem grown and `/home` mounted with no ext4
errors and no emergency mode, zram on, setup started, no user
session, SSH off, multi-user reached, and no failed unit but setup and
gh0st (no keyboard) and `fb_load` (no panel). SSH host keys made on first
start is a device test.

## Changing things

- **Add a package:** add it to `roots.list` with a reason, build, then
  `os/build.sh relock VARIANT` and review the diff of `packages.lock`.
- **Move the Debian snapshot:** change `debian.snapshot` in `lock.json`,
  build, relock, review.
- **Bump the kernel:** change the kernel, header and firmware entries in
  `lock.json` together. `gh0st-panel-modules` rebuilds against the new
  headers and its version follows.
- **Update a stock app:** change its URL, version and SHA-256 in
  `lock.json`. Camera and libcamera move as a pair.

## Spikes

| Spike | Result |
|---|---|
| S1 container | Docker Desktop and mmdebstrap root mode: needs `SYS_ADMIN` plus unconfined seccomp and AppArmor for clean mounts. `mke2fs -d` from the tar keeps owners, setuid bits and hard links. |
| S2 kernel in chroot | Our provider satisfies the kernel; no initramfs-tools. The firmware hooks fill `/boot/firmware`. |
| S3 boot on a clone | To do, on the device. |
| S4 apps without applaunch | To do, on the device. |
| S5 RPi glue | Bluetooth attaches in the kernel (`hci_uart_bcm` from the device tree): no `pi-bluetooth`, no `hciuart`. The udev rules we need from `raspberrypi-sys-mods` are in `gh0st-base`. |
| S6 ffmpeg cost | 148 packages, 367 MB. Kept. |
| S7 AppStore | To do, on the device. |

## Numbers

| | Stock M5 image | Gh0st OS release |
|---|---|---|
| Packages | 1699 | 444 |
| Installed | 5.1 GB `/usr` | 842 MB |
| Download | 14.8 GiB image | 224 MB `.img.xz` |
| Enabled system services | 30+ and 9 timers | 6, one timer (the daily update check) |

## Not done yet

- On the device: the first boot on the spare card, and spikes S3, S4 and S7.
- The archive key and Pages setup (`os/repo/README.md`); the repo itself is
  built (`updates.md`).
- The CI workflow has not run.

## Before the first public release

- A licence and source-offer page: the image redistributes Raspberry Pi
  debs, M5 app debs and the AppStore binary.
- Randomize the disk ID at first boot.
- Tags publish pre-releases until first-boot setup has passed its device
  test on a fresh card (`docs/setup.md`, Tests).
