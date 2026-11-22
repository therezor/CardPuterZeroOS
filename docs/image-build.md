# Building the CPZeroOS image

The image is Debian 13 trixie arm64, built from pinned inputs in a container.
The reasons behind each choice are in `distro-plan.md`; the milestones in
`roadmap.md`. Everything here runs the same on the Mac and on CI.

## Quick start

Needs Docker Desktop running and `jq` on the Mac.

```sh
os/build.sh fetch           # download and verify every pinned input
os/build.sh debs            # build our packages and the drivers
os/build.sh image release   # os/out/cpzeroos-release-YYYYMMDD.img.xz
os/build.sh check release   # tier 1 checks
os/build.sh smoke           # first boot of the dev image in QEMU, about 10 min
```

A clean build takes about ten minutes. `.github/workflows/os.yml` runs the
same steps on an arm64 runner; it has not run yet. Work happens on the Docker volume
`cpzeroos-work`, never on the repo: macOS filesystems lose ownership and
sparse files. Only finished files land in `os/out/`.

## Dev images

A dev image comes up on Wi-Fi with SSH on and no input, so the device is
reachable headless. The secrets reach the container only as environment
variables; they never enter a package, the repo or CI.

```sh
CPZ_DEV_USER=<user> CPZ_DEV_HOSTNAME=<device> \
CPZ_DEV_SSH_PUBKEY="$(cat ~/.ssh/id_ed25519.pub)" \
CPZ_DEV_WIFI_SSID=... CPZ_DEV_WIFI_PSK=... CPZ_DEV_WIFI_COUNTRY=GB \
os/build.sh image dev
```

`CPZ_DEV_PASSWORD_HASH` (from `openssl passwd -6`) sets a password; without
it the account is key-only with passwordless sudo. The dev image also has a
serial console on `serial0` at 115200 that logs in by itself, the
firmware's boot log on the same UART (`uart_2ndstage=1`), a persistent
journal, avahi for the `.local` name, and radios unblocked at boot. A release build fails if any
`CPZ_DEV_*` is set.

## Flash

```sh
diskutil list                                   # find the card: /dev/diskN
scripts/flash.sh os/out/cpzeroos-dev-YYYYMMDD.img.xz /dev/diskN
```

The script refuses internal disks and asks before it writes.

## Layout

```
os/build.sh            host wrapper: builds the container, runs a step
os/Dockerfile          the build container
os/lock.json           every pinned input: URL, SHA-256, commit, snapshot
os/image/build-image.sh  the steps, inside the container
os/image/manifest/     roots.list, deny.list, exec.list, boot-files.list
os/image/variants/     dev and release: extra roots, deny list, the lock,
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
   - `cargo build` for gh0st, gh0st-settings and gh0st-term.
   - The M5 drivers and our panel driver, against the pinned kernel headers.
     M5's `make install` writes to absolute paths, so only `make all` runs
     and an explicit file list goes into the package. Every module's
     vermagic must match the pinned kernel.
3. **image.** `mmdebstrap --variant=minbase` installs `roots.list` from a
   snapshot.debian.org timestamp and the two local repos. Recommends are
   off, and docs, man pages and translations never unpack. The customize
   hook makes UID 1000, applies the variant, runs `systemctl preset-all`
   and removes per-device identity. The boot and home trees come out as
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
repacked alone (`cpzeroos-m5-appstore`): the binary, its images, three fonts
and two sounds. Not the launcher and not `applaunch-updater`, which would
reinstall the stock launcher. It installs apps through `sudo`. It fetches
catalogs from GitHub and an Aliyun mirror and asks ipinfo.io for the
country; that stays until our own Store replaces it.

**ffmpeg.** Files runs `ffmpeg` and `ffprobe` for video previews. Debian's
ffmpeg is a desktop build: its device library pulls OpenGL, so Mesa and
LLVM, and its filters pull speech synthesis and recognition. That is 148
packages and 367 MB of the 838 MB installed. Kept by decision (2026-11-21).

**Packages.**

| Package | Holds |
|---|---|
| `cpzeroos-base` | Config only: apt sources and deny pins, dpkg excludes, unit presets, journald off, zram, sysctl, rfkill default, `/tmp` cap, the first-boot grow, udev rules in place of `raspberrypi-sys-mods`, NetworkManager privacy defaults, PipeWire realtime limits without rtkit, the default volume |
| `cpzeroos-kernel` | Exact-version Depends on the kernel, drivers and firmware, so they move together |
| `cpzeroos-no-initramfs` | Provides `linux-initramfs-tool`: the kernel has ext4 and the SD driver built in |
| `cpzeroos-panel-modules` | The 15 M5 drivers and our panel driver, overlays, udev rules, firmware files, keymap, `fb_load` |
| `gh0st`, `gh0st-term` | Our binaries, `.desktop` files, polkit files, the `gh0st.service` user unit |
| `cpzeroos-m5-appstore` | See above |
| `cpzeroos-dev` | Dev only: SSH, avahi, sudo without a password, journal, tools |

**Services.** System: NetworkManager and its on-demand dispatcher,
timesyncd, bluetoothd, `fb_load` and the first-boot grow; the dev image adds
sshd and avahi, and logs the serial console straight in. User: gh0st, and
PipeWire's sockets. No timers: the tmpfiles timer is masked, `/tmp` is a
tmpfs. `units-*.expected` in each variant is the list the checks hold it to.

## Kernel and boot

- No initramfs. Root is found by PARTUUID.
- `raspi-firmware` and the kernel hooks fill `/boot/firmware` in the chroot.
  The boot partition then keeps only what `boot-files.list` names: the
  CM0's firmware, kernel and dtb, and all overlays. Our four overlays are
  copied in by `cpzeroos-panel-modules`.
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

All mount `noatime,commit=60`. The disk ID is fixed (`0x43505a30`, "CPZ0"),
so every card shares PARTUUIDs; randomizing it at first boot is still to do.
On first boot `cpzeroos-grow-data.service` grows p3, and `x-systemd.growfs`
grows its filesystem. machine-id and the random seed are made by systemd.
SSH host keys are made the first time SSH starts, by an `ExecStartPre`
drop-in: Debian's `sshd-keygen.service` only runs on systemd's first boot.

## Checks

`os/build.sh check VARIANT` runs offline on the last image:

- The packages equal `packages.lock`. Our own packages' versions are masked.
- Nothing from the deny lists. Every `+rpt` package is pinned in `lock.json`.
- apt's manual packages are `roots.list` plus Debian's required base.
- Enabled units equal `units-*.expected`. No timers.
- No SSH host keys, empty machine-id, no random seed, no apt lists, and no
  Wi-Fi profiles in a release.
- Every overlay `config.txt` names is on the boot partition, and nothing for
  other boards.
- UID 1000's home sits at the root of the data partition.
- setuid bits survive `mke2fs -d`; the helper is a hard link.
- Every command in `exec.list` resolves. Our binaries are static, so no
  package metadata says what they run.
- Stock app and AppStore libraries resolve (`ldd`).
- Driver vermagic matches the kernel; depmod finds no unknown symbols.
- Package count and installed size stay within `budget.conf`.

## Smoke test

`os/build.sh smoke` boots the dev image in QEMU's `raspi3b`, a Pi 3 close to
the CM0, for ten minutes. QEMU has no panel, keyboard, Wi-Fi or VideoCore,
so it proves the boot path only. Its serial console stays silent with this
device tree, so the test reads the dev image's journal off the card file
afterwards. It checks: root mounted without an initramfs, p3 and its
filesystem grown, zram on, SSH host keys made and sshd listening, UID 1000's
session started, multi-user reached, no failed unit but gh0st (no keyboard),
and a writable home.

## Changing things

- **Add a package:** add it to `roots.list` with a reason, build, then
  `os/build.sh relock VARIANT` and review the diff of `packages.lock`.
- **Move the Debian snapshot:** change `debian.snapshot` in `lock.json`,
  build, relock, review.
- **Bump the kernel:** change the kernel, header and firmware entries in
  `lock.json` together. `cpzeroos-panel-modules` rebuilds against the new
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
| S5 RPi glue | Bluetooth attaches in the kernel (`hci_uart_bcm` from the device tree): no `pi-bluetooth`, no `hciuart`. The udev rules we need from `raspberrypi-sys-mods` are in `cpzeroos-base`. |
| S6 ffmpeg cost | 148 packages, 367 MB. Kept. |
| S7 AppStore | To do, on the device. |

## Numbers

| | Stock M5 image | CPZeroOS release |
|---|---|---|
| Packages | 1699 | 440 |
| Installed | 5.1 GB `/usr` | 838 MB |
| Download | 14.8 GiB image | 224 MB `.img.xz` |
| Enabled system services | 30+ and 9 timers | 6, no timers |

## Not done yet

- On the device: the first boot on the spare card, and spikes S3, S4 and S7.
- Signing, the apt repo on Pages, release assets (roadmap M6).
- The CI workflow has not run.

## Before the first public release

- A licence and source-offer page: the image redistributes Raspberry Pi
  debs, M5 app debs and the AppStore binary.
- Randomize the disk ID at first boot.
- Tags publish pre-releases only until first-boot setup (roadmap M4): the
  release account is locked and there is no setup yet.
