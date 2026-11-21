# Roadmap

This is the path from a shell on a stock image to CPZeroOS as its own OS. The
research and the reasons behind each choice are in `distro-plan.md`. Each
milestone ends with a test on the device, measured the same way before and
after.

## The model

Everything is an app. The shell (`gh0st shell`) is the launcher plus the OS
chrome: the top bar, an optional bottom bar, ESC hold, the global keys, low
battery and the lock. Settings, the terminal, the Store and the camera are
apps. Apps reach the shell through the link in `apps.md`, Rust apps through the
SDK in `sdk.md`. The root helper (M3) does what needs root.

## Decisions

The user made these on 2026-11-20 and 2026-11-21.

- **Threats.** CPZeroOS defends against hostile networks, a lost or stolen
  device, and network observers who see DNS and NTP. Untrusted apps come in
  stages (see below).
- **Encryption.** None for now. Anyone with the SD card can read it, and the
  lock screen only stops someone using the device itself. User data gets its
  own partition (M6), so L1 can encrypt that partition alone.
- **Updates.** Our packages come from a signed apt repo, checked only when the
  user asks. The kernel and the M5Stack modules move together as one pinned
  pair.
- **Isolation.** Each app runs in its own cgroup.
- **Settings.** Settings is its own crate and binary, `gh0st-settings`, built
  on the SDK.

## What is missing

None of the phases in `distro-plan.md` is built. There is no image build and
no system baseline. The dev device runs the stock desktop image, with LightDM,
avahi, rpcbind, udisks2 and cron up, and 93 MB in swap at idle.

These gaps were in no doc before this one:

1. **Data at rest.** See Decisions. The CPU has no AES instructions, so
   encryption will use Adiantum, which the rpt kernel has as a module.
2. **Lock auth.** Holding Tab locks the screen, with no PIN or password.
3. **App isolation.** Every app runs as UID 1000 and can use the network, the
   camera, the mic and the home directory.
4. **Memory pressure.** zram exists, but nothing decides which app dies first.
   memcg and PSI are both off today.
5. **Network hardening.** Nothing drops inbound traffic, and sshd, rpcbind and
   avahi listen. There are no IPv6 privacy addresses, the DHCP IDs identify the
   device, and the user can't choose a DNS server.
6. **Per-device identity.** machine-id, the SSH host keys and the random seed
   must be made at first boot, not baked into the image.
7. **Updates and packaging.** There are no `.deb`s and no signed repo, and
   nothing checks the Store catalogs.
8. **Root helper.** Logs, SSH, time, the password and updates need root. One
   helper with fixed verbs covers them all.
9. **Panel driver delivery.** `install.sh` builds the driver on the device, and
   the image drops the compilers.
10. **Deep idle and power loss.** The kernel has no suspend. Nothing shuts down
    cleanly when power fails (GPIO6).
11. **Recovery.** There is no partition layout and no factory reset. Nothing
    brings the device back when the shell won't start and SSH is off.
12. **Release gates.** Nothing measures the whole system, checks for open
    ports, or captures idle traffic.
13. **Stock apps.** Nobody has audited their traffic.
14. **Screen-off and lock over an app.** Both work only on the home screen.

## Untrusted apps, in stages

Every app runs as UID 1000, the same user as the shell. A hostile app can
ptrace the shell (the kernel has no Yama), call the root helper, and likely
move itself to another cgroup. So the first step stops apps that phone home by
habit. It doesn't stop hostile ones.

| Step | How | Effort | When |
|---|---|---|---|
| Network off per app | The root helper adds an nftables `socket cgroupv2` drop rule for the app's cgroup before exec | Low | M5 |
| Hide private files | A mount namespace in the child before exec, with tmpfs over `~/.ssh` and other apps' data | Medium | L3 |
| Camera per app | A BPF device program on the app's cgroup | Medium | L3 |
| Mic per app | PipeWire client permissions | Hard | L3 |
| Enforce network and helper rules | Root-owned app cgroups, Yama, and a UID per app or Landlock | High | L3 |
| Store `postinst` runs as root | The Store installs only from our signed catalog | Hard | M6 |

## Milestones

The OS track (M) and the app track (A) run side by side. OS work ships as
`.deb`s and runs on the dev device first, so the image (M6) only assembles
packages.

**M0. System baseline and app audit.**
- Add a `system` mode to `scripts/measure.sh`. It records MemAvailable, CMA,
  swap, the top PSS, running services, `ss -tulpn`, `systemd-analyze`, IRQ and
  context switch deltas, and `current_now` with the screen on and off.
- Capture 10 minutes of idle traffic on the AP. Then run each stock app under
  the same capture. The hosts each app contacts set the M5 network defaults.
- Done when `hardware-facts.md` has a stock column and `docs/` has an app
  traffic table.

**M1. Performance config package.** `cpzeroos-base` holds config only, as
conf.d drop-ins, next to a `gh0st` package.
- Turn off LightDM, labwc, avahi, rpcbind, nfs, udisks2, accounts-daemon, cron
  and the timers.
- Set `gpu_mem=64`. Stock `rpi-swap` already runs zram, plus a writeback file
  on the SD card and a timer. Keep the zram and drop the writeback, which wakes
  the CPU and wears the card. Tune the zram size and `swappiness`.
- Cap the `/tmp` tmpfs at 32M, set journald to `Storage=none`, and add the USB
  autosuspend rule, matched by VID:PID.
- Measure memcg (`cgroup_enable=memory`) and PSI (`psi=1`) before turning them
  on. They cost RAM and CPU, and only M5 needs them.
- After 3 crashes the `gh0st` unit falls back to stock APPLaunch
  (`OnFailure=`). Once M4 exists, the fallback opens on a locked screen.
- `cpzeroos-panel-modules` holds our panel driver and the M5Stack modules,
  built in CI against the pinned kernel headers.
- `cpzeroos-dev` keeps SSH on and port 22 open for the device loop.
- Hide ZClaw under stock APPLaunch too (`preinstalled-desktop-apps.tsv`). It
  contacts Cloudflare and Microsoft at start. gh0st already hides it.
- Done when each change is measured against M0, and the panel, keyboard, audio
  and camera still work.

**M1b. Image spike.** Build a minbase image on a second SD card, by hand if
needed, from mmdebstrap, the pinned kernel, the M5Stack modules, our packages
and `cpzeroos-dev`. It finds the driver, `fb_load`, PipeWire and library
problems early.
- Target hardware rev v0.5, as M5Stack's CI does (`cardputerzero-v5-overlay`,
  `bq27220_v5`, `bmi270_bmm150_overlay`). Check that the same image boots on
  the Lite SKU, where the camera and IMU probes should fail harmlessly.
- Ship what stock apps need at runtime: NetworkManager, BlueZ, PipeWire-pulse,
  polkit with `60-applaunch-updater.rules` (until M2 decides), and `fb_load`.
- The M2 and M4 tests run on this card. They turn SSH off and drop inbound
  traffic, which would lock us out of the dev card.
- Done when it boots to gh0st and stock apps run.

**A1. App SDK v0.** `crates/gh0st-app` with pages, OS bars, soft keys, lists,
text fields, jobs, timers, a sim backend and a test driver. Built; the guide is
`sdk.md`. Still to do: the shell link (`apps.md`), so the shell draws the bars
instead of each app. Done when an SDK app runs on the device and on the Mac,
and its soft keys line up with keys 4 to 8.

**A2. Settings v0.** `gh0st-settings` with the pages that need no root:
Appearance, Display, Sound and About. Built and run in the sim. Done when each
setting applies on the device and the app has zero wakeups while idle.

**M2. Network and privacy defaults**, in `cpzeroos-base`.
- nftables drops inbound traffic, except established connections, DHCP and
  ICMPv6 ND. SSH is open only while the user has it on.
- NetworkManager uses a random or stable-per-SSID MAC and randomizes the scan
  MAC. It sets `dhcp-send-hostname=false`, `ip6-privacy=2` and
  `stable-privacy`, and runs no connectivity check.
- NetworkManager gets `auth-polkit=root-only`. Settings has no logind session,
  so network changes go through the root helper. This also stops apps changing
  the MAC or DNS policy.
- systemd-resolved sends DNS over TLS to a resolver the user picks, with LLMNR
  and mDNS off. It is a resident daemon, so measure its cost.
- The hostname is neutral, the time server is configurable, and Bluetooth is
  off and hidden.
- sysctl sets `dmesg_restrict`, `kptr_restrict` and
  `core_pattern=|/bin/false`.
- WireGuard is an option, through NetworkManager.
- Measure iwd against wpa_supplicant.
- Done when, with Wi-Fi up and SSH off, `ss -tulpn` is empty, a LAN scan finds
  nothing, and the AP capture shows only DHCP, DoT to the chosen resolver, and
  NTP.

**M3. Root helper.** `gh0st-helper` lives in the gh0st binary. systemd starts
it on demand from a socket, and it exits when idle.
- It checks `SO_PEERCRED` for UID 1000, takes fixed verbs with checked
  arguments, and never runs a shell.
- Verbs: logging mode, SSH, ADB, hostname, timezone, time, NTP server,
  regdomain, password, rfkill, Wi-Fi and network, reboot, power off, updates,
  factory reset, and per-app network rules.
- Any app can reach the socket as UID 1000. So the sensitive verbs (SSH, ADB,
  password, factory reset, updates and network rules) carry the user's
  password, which the helper checks and rate limits.
- Done when the verb parser has unit tests and no helper process runs while
  idle.

**A3. Settings, root pages.** Wi-Fi, Network, Bluetooth, Privacy (logs Off,
RAM or Persistent, MAC policy, SSH, ADB, radios, DNS), Time and Updates.

**M4. First boot, identity, lock.**
- Setup runs on the device: a keyboard tutorial, then user, password, Wi-Fi,
  timezone, regdomain and SSH opt-in. The device uses no network until the user
  joins one.
- The image ships an empty machine-id. SSH host keys are made when the user
  first turns SSH on. The random seed is made at first boot.
- The lock screen asks for the password, checked by the helper, with a growing
  delay after each wrong try.
- Screen-off and lock work while an app runs too. Stock Camera grabs the
  keyboard, so idle can't come from key events alone.
- Factory reset formats the user data partition and returns to setup.
- Done when a reset device boots to setup with no open ports and no traffic.

**M5. Apps in cgroups, multitasking, memory.**
- gh0st's unit gets `Delegate=yes`. The shell moves itself to a leaf cgroup and
  starts each app in its own child cgroup.
- Spike first. Can an app move itself out of its cgroup and dodge its network
  rule? Which PSI trigger window may an unprivileged process set?
- The shell freezes background apps with `cgroup.freeze`, then
  `memory.reclaim` pushes their pages to zram. The switcher is a plain list.
- OOM order: the shell lowest, apps higher, frozen apps highest. Under PSI
  pressure the oldest frozen app dies first.
- Each app has a network toggle, set through the helper.
- Done when three frozen apps show their PSS drop, a memory hog kills a frozen
  app and never the shell, and an app with network off can't connect.

**M6. Image, CI, updates.**
- The image is mmdebstrap trixie minbase, the RPi archive with the kernel held,
  and our packages.
- Partitions: FAT boot, ext4 root, and ext4 user data that grows at first
  boot. All mount with `noatime,commit=60`.
- GitHub Actions builds `.img.xz` and `.bmap`, and a signed apt repo for our
  packages.
- Decide how the Store handles catalogs and `postinst`.
- `scripts/gate.sh` checks RAM, boot time, idle current, open ports and the
  packet capture, and fails on a regression. It also fails if `cpzeroos-dev` is
  in a release. The targets come from M0 and M1.
- Done when a fresh flash passes every gate.

**M7. Boot and splash.** gh0st draws the splash and `fb_load` goes. No
initramfs, `quiet loglevel=3`, `boot_delay=0`, tuned with `systemd-analyze
critical-chain`. Done when the time to the home screen is gated.

**M8. Deep idle and power loss.** With the screen off, apps freeze, the panel
sleeps, Wi-Fi saves power, audio suspends, and a key wakes the device. When
power fails (GPIO6), the device syncs and shuts down. Done when screen-off
current is measured against the 173 mA floor.

**Later.**
- L1: fscrypt with Adiantum on the user data partition.
- L2: our own kernel, trimmed, with the M5Stack drivers in-tree, Landlock, and
  Adiantum built in.
- L3: untrusted apps, stage 2 (the table above).
- L4: desktop mode as an app (`distro-plan.md` §6).
