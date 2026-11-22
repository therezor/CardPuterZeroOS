# Roadmap

This is the path from a shell on a stock image to Gh0st OS as its own OS. The
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

- **Threats.** Gh0st OS defends against hostile networks, a lost or stolen
  device, and network observers who see DNS and NTP. Untrusted apps come in
  stages (see below).
- **Encryption.** None for now. Anyone with the SD card can read it, and the
  lock screen only stops someone using the device itself. User data gets its
  own partition (M6), so L1 can encrypt that partition alone.
- **Updates.** Our packages come from a signed apt repo on GitHub Pages,
  stable and edge. The device checks once a day (on by default, off in
  Settings or setup) and installs only when the user confirms. The kernel
  set moves as one, and only above 50% battery. Changed 2026-11-22 from
  "checked only when the user asks"; `updates.md` has the design.
- **Isolation.** Each app runs in its own cgroup.
- **Settings.** Settings is its own crate and binary, `gh0st-settings`, built
  on the SDK.

## What is missing

The image build exists (`image-build.md`, 2026-11-21): M1b's image and most of
M6's build and CI, not yet booted on a device. There is no system baseline. The dev device runs the stock desktop image, with LightDM,
avahi, rpcbind, udisks2 and cron up, and 93 MB in swap at idle.

These gaps were in no doc before this one:

1. **Data at rest.** See Decisions. The CPU has no AES instructions, so
   encryption will use Adiantum, which the rpt kernel has as a module.
2. **Lock auth.** Built 2026-11-22: the lock screen asks for the password
   (`design.md`, The lock screen). Still unmeasured on the device.
3. **App isolation.** Every app runs as UID 1000 and can use the network, the
   camera, the mic and the home directory.
4. **Memory pressure.** zram exists, but nothing decides which app dies first.
   memcg and PSI are both off today.
5. **Network hardening.** Nothing drops inbound traffic, and sshd, rpcbind and
   avahi listen. There are no IPv6 privacy addresses, the DHCP IDs identify the
   device, and the user can't choose a DNS server.
6. **Per-device identity.** machine-id, the SSH host keys and the random seed
   must be made at first boot, not baked into the image.
7. **Updates and packaging.** Built 2026-11-22 (`updates.md`): the engine,
   the Updates app, notifications and the Pages repo. Still open: the key
   and Pages setup, the device test, and nothing checks the Store catalogs.
8. **Root helper.** `gh0st-helper` exists, run by pkexec. Our polkit rule
   lets anyone in `sudo` or `netdev` run it, and control NetworkManager,
   without a password. The verbs that matter (SSH, logs, GPU memory, Wi-Fi
   country, CPU mode, hostname, swap) now check the user's password themselves.
   Still open: power and rfkill need none, NetworkManager and timedate are
   allowed outright, and an app as UID 1000 can ptrace Settings or read the
   keyboard while the password is typed. The try counter lives in `/run`, so
   a reboot resets it.
9. **Panel driver delivery.** `install.sh` builds the driver on the device, and
   the image drops the compilers.
10. **Deep idle and power loss.** The kernel has no suspend. Nothing shuts down
    cleanly when power fails (GPIO6).
11. **Recovery.** There is no partition layout and no factory reset. Nothing
    brings the device back when the shell won't start and SSH is off.
12. **Release gates.** Nothing measures the whole system, checks for open
    ports, or captures idle traffic.
13. **Stock apps.** Nobody has audited their traffic.
14. **Screen-off and lock over an app.** Built 2026-11-22: dark pauses the
    app in front (`apps.md`, Pausing). Still unmeasured on the device.

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

## Settings: what comes next

**Hotspot.** Built 2026-11-22 (`hotspot.md`). Still to come: turn it off
when nobody has joined for a while (iw's station events, no polling), and
its device checks. Its verbs are ungated, like the NetworkManager grant they
stand beside; M3 gates both.

In order. Each row names what it changes and whether it works on today's
stock image. Rows marked "after M3" need the helper's password check, so
their new verbs must be gated (`Verb::gated` in `helper.rs`).

**Advanced page** (System > Advanced). Built 2026-11-21; the device checks
still open are in `hardware-facts.md`. A value is picked on its own page,
each option with a note. Only then does it ask for the user's password,
which holds for 5 minutes in the same Settings process.
A change that needs a restart offers one, and the page shows "Restart to
apply" until the running value matches the file.

| Setting | What it changes | Helper verb | Applies |
|---|---|---|---|
| GPU memory | `gpu_mem` in `config.txt`: 16, 32, 64, 96, 128, 192 or 256 MB | `gpu-mem 64` | Restart |
| Wi-Fi country | `cfg80211.ieee80211_regdom` in `cmdline.txt` | `regdom XX` | Restart |
| CPU mode | The governor: schedutil, powersave or performance, kept by a udev rule after stock's | `cpu balanced`, `power-saver` or `performance` | Now |
| Device name | The hostname and its `/etc/hosts` line | `hostname NAME` | Now |
| Swap size | The zram size in MB, typed; 0 is off, at most twice RAM. A zram-generator drop-in (`zram-size`), or on stock an rpi-swap one (`FixedSizeMiB`, `Mechanism=none` for off) | `swap 256` | Restart |
| Swap compression | `compression-algorithm` in a zram-generator drop-in, on both systems: zstd, lz4 or lzo-rle | `swap-comp lz4` | Restart |
| Swappiness | `vm.swappiness` in a sysctl.d drop-in, and in `/proc` at once: 10, 60, 100, 150 or 200 | `swappiness 150` | Now |

**Privacy page.** Today it has one row, Logs.

| Setting | What it changes | Helper verb | Works today |
|---|---|---|---|
| What is exposed | A readout at the top: open ports from `ss -tln`, SSH on or off, radios on | None, read only | Yes |
| Wi-Fi MAC for new networks | `wifi.cloned-mac-address` (random or stable per network) and scan MAC randomization, in a NetworkManager conf.d file | `mac random`, `stable` or `device` | Yes |
| Send hostname to DHCP | The `ipv4.dhcp-send-hostname` and `ipv6.dhcp-send-hostname` defaults, in the same file | `dhcp-hostname on` or `off` | Yes |
| IPv6 privacy addresses | The `ipv6.ip6-privacy=2` default | `ip6-privacy on` or `off` | Yes |
| Bluetooth visible | `bluetoothctl discoverable off`, and a neutral device name | None | Yes |
| Clear logs now | Vacuums the journal | `logs clear` | Yes |
| Forget all Wi-Fi networks | `nmcli connection delete` for each Wi-Fi profile, after a confirm | None | Yes |
| Time server | A timesyncd conf.d file: the Debian pool, a typed server, or off | `ntp-server` | Yes |
| SSH keys only | An sshd_config.d drop-in with `PasswordAuthentication no` | `ssh-keys on` or `off`, after M3 | Yes |
| ADB | The stock ADB gadget unit | `adb on` or `off`, after M3 | Yes |
| Encrypted DNS | systemd-resolved with DNS over TLS to Quad9, Cloudflare or a typed server | `dns`, after M3 | Needs systemd-resolved (M2) |
| Firewall | The nftables inbound policy from M2, shown and switched | `firewall on` or `off`, after M3 | Needs nftables (M2) |
| Network per app | In Apps, per app: network allowed or blocked | `net-block`, after M3 | Needs cgroups (M5) |

**Lock and identity** (M4).
- Screen lock: Unlock with password, and Lock after (at once, 1, 5 or 15
  minutes of dark, or only by hand; `gh0st.lock.after`, 5 minutes by
  default). The shell checks the password itself (M4).
- Factory reset: formats the user data partition, then runs first-boot setup.

**Other pages.**
- Keyboard: key repeat delay and rate, which the shell reads from config.
- Storage: used and free, the screenshots folder, and clearing the apt cache
  (helper verb `apt-clean`).
- Battery: charge, health and cycle count from the gauge, and the
  low-battery power-off level.
- Date and time: set by hand when network time is off
  (`timedatectl set-time`).
- Updates: built 2026-11-22 (System > Updates, `updates.md`).

**In the SDK and the shell.**
- The shell link (A1), so the shell draws Settings' bars and Settings keeps
  no clock timer.
- A dot in the top bar while the camera or the mic is in use. The shell draws
  it, so no app can hide it.

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

**M1. Performance config package.** `gh0st-base` holds config only, as
conf.d drop-ins, next to a `gh0st` package.
- Turn off LightDM, labwc, avahi, rpcbind, nfs, udisks2, accounts-daemon, cron
  and the timers.
- Set `gpu_mem=64` (Settings > System > Advanced can switch it back to 128).
  Stock `rpi-swap` already runs zram, plus a writeback file
  on the SD card and a timer. Keep the zram and drop the writeback, which wakes
  the CPU and wears the card. Tune the zram size and `swappiness`.
- Cap the `/tmp` tmpfs at 32M, set journald to `Storage=none`, and add the USB
  autosuspend rule, matched by VID:PID.
- Measure memcg (`cgroup_enable=memory`) and PSI (`psi=1`) before turning them
  on. They cost RAM and CPU, and only M5 needs them.
- After 3 crashes the `gh0st` unit falls back to stock APPLaunch
  (`OnFailure=`). Once M4 exists, the fallback opens on a locked screen.
- `gh0st-panel-modules` holds our panel driver and the M5Stack modules,
  built in CI against the pinned kernel headers.
- Hide ZClaw under stock APPLaunch too (`preinstalled-desktop-apps.tsv`). It
  contacts Cloudflare and Microsoft at start. gh0st already hides it.
- Done when each change is measured against M0, and the panel, keyboard, audio
  and camera still work.

**M1b. Image spike.** Build a minbase image on a second SD card, by hand if
needed, from mmdebstrap, the pinned kernel, the M5Stack modules, our packages
It finds the driver, `fb_load`, PipeWire and library
problems early.
- Target hardware rev v0.5, as M5Stack's CI does (`cardputerzero-v5-overlay`,
  `bq27220_v5`, `bmi270_bmm150_overlay`). Check that the same image boots on
  the Lite SKU, where the camera and IMU probes should fail harmlessly.
- Ship what stock apps need at runtime: NetworkManager, BlueZ, PipeWire-pulse,
  polkit with `60-applaunch-updater.rules` (until M2 decides), and `fb_load`.
- The M2 and M4 tests run on this card. They turn SSH off and drop inbound
  traffic, which would lock us out of the dev card.
- Done when it boots to gh0st and stock apps run.
- Built 2026-11-21 (`image-build.md`). Since 2026-11-22 there is one image,
  with no dev variant: it runs first-boot setup. It passes the offline checks;
  the boot on the spare card is next.

**A1. App SDK v0.** `crates/gh0st-app` with pages, OS bars, soft keys, lists,
text fields, jobs with stdin, timers, watched fds, raw keys, a sim backend
and a test driver. Built; the guide is `sdk.md`. The shell link carries
`hello`, `pause`, `resume`, `redraw` and `toast`. Still to do: the bars over
the link (`apps.md`), so the shell draws them instead of each app. Done when
an SDK app runs on the device and on the Mac, and its soft keys line up with
keys 4 to 8.

**A2. Settings as an app.** Done in the sim on 2026-11-21. Every page the
shell had (Wi-Fi, Network, a connection, Bluetooth, Display, Sound,
Appearance, Apps, one app, System, time zones, Privacy, About) moved into
`gh0st-settings` with its tests. The shell lost about 3,000 lines and 86 KB of
binary. Done when each page works on the device. The next steps are in
Settings: what comes next.

**A3a. The terminal.** `gh0st-term`, built in the sim on 2026-11-21 (branch
`terminal`): tabs, fit or a window over a larger grid, Terminus at five
sizes, history with search, prompt jumps and line
copy, and running behind other apps on the shell link. The guide is
`terminal.md`. Done when its budgets hold on the device:
- no wakeup at idle with the top bar off; with it on, the clock's minute and
  the battery read every 2 s;
- PSS at most `gh0st-settings` + 1 MB with one tab and empty history;
- history at most 80 bytes per line of shell text;
- a key echoed within one frame, 25 ms;
- `cat` of 50 MB within 1.5x of the same into a PTY with a reader that
  discards, with keys still answered;
- behind another app: no draw, no flush, and the shell's wakeups at home as
  without it.

Still to come: the kitty keyboard protocol, a visual bell, and a check of
which Fn combos the keyboard passes clean (`hardware-facts.md`).

**M2. Network and privacy defaults**, in `gh0st-base`.
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

**M3. Root helper, locked down.** `gh0st-helper` lives in the gh0st binary
and runs through pkexec today. It runs only when called and exits after.
- It takes fixed verbs with checked arguments and never runs a shell.
- Our polkit rule says YES, without a password, to anyone in `sudo` or
  `netdev`. pkexec can't ask for a password here, since no polkit agent runs.
  So the helper checks the password itself, read from stdin. Built
  2026-11-21 with the Advanced page; wrong, missing and locked passwords are
  measured on the device, a right one and the grace window are still to
  check (`hardware-facts.md`): `unix_chkpwd` checks it (yescrypt, which static
  musl can't), three free tries then a wait that doubles to 5 minutes, and a
  correct password holds for 5 minutes in the same calling process.
- Gated today: SSH, logs, GPU memory, regdomain, CPU mode, hostname and swap. To
  come, also gated: firewall, DNS, ADB, factory reset, updates and network
  rules.
- Verbs still to add: ADB, timezone, time, NTP server, password, Wi-Fi and
  network, updates, factory reset, and per-app network rules.
- Done when the verb parser has unit tests and no helper process runs while
  idle.

**A3. Settings, privacy and the rest.** See Settings: what comes next.

**M4. First boot, identity, lock.**
- Setup runs on the device, as root before any user session: welcome, keyboard
  tutorial, country and time zone, privacy review, network, date and time when
  not synced, account (name and a required password), device name and look.
  The device uses no network until the user joins one. Designed 2026-11-21 in
  `setup.md`.
- The image ships an empty machine-id. SSH host keys are made when the user
  first turns SSH on. The random seed is made at first boot.
- The lock screen asks for the password, checked with `unix_chkpwd`, with a
  growing delay after each wrong try. Settings' Unlock with password turns
  that off. The shell runs `unix_chkpwd` itself, not through the helper: the
  helper's 5-minute grace would let a second unlock through with no
  password. The try count lives in `$XDG_RUNTIME_DIR`, so a shell restart
  keeps the wait. The shell starts locked, so a restart or a power cycle
  doesn't skip it. A dark longer than Lock after ends on the lock screen,
  which takes keys only after Tab then Enter, so a pocket can't type into an
  app or the password. With no usable password, Enter unlocks: a lockout would
  mean a reflash.
- Screen-off and lock work while an app runs too. Dark pauses the app in
  front, as Fn+K does. An app that grabs the keyboard hides the keys from
  the shell, so the shell tries the grab first and keeps the screen on when
  it can't take it. `design.md` has the lock screen and the dark state.
- Factory reset formats the user data partition and returns to setup.
- Done when a reset device boots to setup with no open ports and no traffic.

**M5. Apps in cgroups, multitasking, memory.**
- Done on the `switcher` branch: Fn+K pauses the app in front with SIGSTOP
  and opens the switcher, Settings > Apps lets an app run in the background,
  and the shell grabs the keyboard while apps are paused.
- gh0st's unit gets `Delegate=yes`. The shell moves itself to a leaf cgroup and
  starts each app in its own child cgroup.
- Spike first. Can an app move itself out of its cgroup and dodge its network
  rule? Which PSI trigger window may an unprivileged process set?
- The shell freezes background apps with `cgroup.freeze`, then
  `memory.reclaim` pushes their pages to zram.
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
  packages (built 2026-11-22: `apt.yml`, `os/repo`).
- Decide how the Store handles catalogs and `postinst`.
- `scripts/gate.sh` checks RAM, boot time, idle current, open ports and the
  packet capture, and fails on a regression. The targets come from M0 and M1.
- Done when a fresh flash passes every gate.

**M7. Boot and splash.** gh0st draws the splash and `fb_load` goes. No
initramfs, `quiet loglevel=3`, `boot_delay=0`, tuned with `systemd-analyze
critical-chain`. Done when the time to the home screen is gated.

**M8. Deep idle and power loss.** There is no suspend (`PM_SLEEP` is off),
so deep idle is the dark state made cheaper. M4 pauses the app in front and
stops every shell timer. Built 2026-11-22: USB power saving and Wi-Fi off in
the dark (`hardware-facts.md`, Dark current). Panel sleep measured at about
7 mA, within noise, so it waits. When
power fails (GPIO6), the device syncs and shuts down. Done when screen-off
current is measured against the 173 mA floor.

**Later.**
- L1: fscrypt with Adiantum on the user data partition.
- L2: our own kernel, trimmed, with the M5Stack drivers in-tree, Landlock, and
  Adiantum built in.
- L3: untrusted apps, stage 2 (the table above).
- L4: desktop mode as an app (`distro-plan.md` §6).
