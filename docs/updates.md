# Updates

A device updates from apt, without a reflash. Debian packages come from
Debian. Ours, the Raspberry Pi set and the stock apps come from our signed
repo on GitHub Pages. The device checks once a day and tells the user. It
installs only when the user says so.

The pictures come from `crates/gh0st-ui/examples/updates.rs`:

```sh
cargo run -p gh0st-ui --example updates   # writes mockups/updates
```

## What the user decided

2026-11-22:

- Check daily by default. Install only after the user confirms. There is no
  mode that installs on its own.
- Two channels: stable, from version tags, and edge, from every push to main.
- The kernel set updates like any other package, but only with the battery
  above 50%.
- Install and the update settings ask for the password, once per 5 minutes.
  Checking does not.

This replaces "updates only when the user asks" from 2026-11-20.

## How it fits together

```
daily timer ─────────┬──> gh0st-update-daily.service ──> gh0st update check --auto
network up (pending) ┘
Updates app, Settings ──> gh0st-update-check.service ──> gh0st update check
   (gh0st-helper)    └──> gh0st-update-install.service ─> gh0st update install
                                                           └─ apt-get, Status-Fd 3
                     writes ─> /var/lib/gh0st/update/state     what is new
                               /run/gh0st-update/progress      how far along
                               /var/lib/gh0st/notify/          the notice
the Updates app, Settings and the shell read those with inotify
```

apt runs in a system unit, never as a job of the app:

- A job shares the app's process group. Holding Esc, or closing the app,
  would cut its pipe, and dpkg dies on SIGPIPE in the middle of an install.
- Restarting the shell after a gh0st upgrade kills every process in its
  cgroup, the app included.
- The shell can't pause an app with a root child.
- The daily check has no screen and no password.

So the app only shows what the unit does. It can close, pause or crash, and
the install carries on. Opening it again shows where it is.

## The engine

`gh0st update check` and `gh0st update install`, in the multi-call binary.
Only systemd runs them, as root.

**Check.**

1. Takes `/run/gh0st-update/lock`. A second run waits for the first.
2. With no default route it records `offline`, leaves the pending mark and
   stops.
3. The daily run skips a metered link (a phone hotspot) and stays pending. A
   check by hand doesn't.
4. A clock earlier than the image's build time records `clock`: apt would
   reject every Release file as "not valid yet".
5. `apt-get update`, with progress from `APT::Status-Fd`.
6. `apt-get -s upgrade --with-new-pkgs` lists what would change. One
   `apt-cache show` adds sizes and summaries.
7. Each package lands in a group: Gh0st OS (origin Gh0st), Security
   (Debian-Security) or System. It also gets a restart: the device for the
   kernel set, `libc6` and `systemd`, Gh0st for `gh0st` and `gh0st-term`.
8. Writes the state and posts, replaces or removes the notice.
9. The daily run then refreshes the Store's app list under the same lock,
   and the Store posts its own notice, "3 app updates" (`docs/store.md`).

**Install.**

1. Takes the lock.
2. Lists older than an hour are refreshed first, since Debian removes
   superseded debs. A 404 or a hash mismatch refreshes once and retries.
3. Battery: at least 30% or charging. With the kernel set in the list,
   **above 50%**, charging or not: it rewrites `/boot/firmware`, and a power
   cut there can stop the device booting.
4. Space: the download, plus what grows on disk, plus 100 MB.
5. An interrupted dpkg (`/var/lib/dpkg/updates` not empty) is finished first
   with `dpkg --configure -a`.
6. `apt-get -y upgrade --with-new-pkgs` with Status-Fd progress,
   `--force-confdef --force-confold`, no prompts, and a 120 s wait for the dpkg
   lock. When the Store holds that lock, the result is "Another install is
   running".
7. Pending migrations, below.
8. `sync`, `apt-get clean`, then the check's listing again.

SIGTERM stops a download. Once dpkg starts, the engine finishes first.

**Migrations.** A change an update makes once on each device, then never
again. gh0st-base ships them in `/usr/share/gh0st/migrations`, a file each,
run in name order. A line `install curl wget` adds packages that a device
flashed before them lacks. A line `remove gh0st-m5-appstore` takes off one
the image no longer ships, since `apt-get upgrade` never removes anything.

- The check lists what pending migrations would add, as updates, so the user
  sees and installs it like any other.
- The install runs them after the upgrade, which may have just brought them,
  checking power and space again. The packages go in as installed by hand: the
  user may remove one later, and nothing brings it back.
- A removal runs first, and only when apt's simulation removes nothing
  else; otherwise it waits and the log says why.
- A migration whose packages are all there, and whose removed ones are
  gone, is done. Its name goes in `/var/lib/gh0st/update/migrations`.
- A failed install leaves it pending for the next one. A package the archive
  lacks makes it wait, and never fails a check.
- A fresh install of gh0st-base, which is a new image, marks every migration
  it ships done.

A device that upgrades from an engine without migrations runs them on the
install after the next check.

**Files.** Plain `key=value` lines, with one tab-separated line per package.
They are read by the engine, the app, Settings and the shell
(`gh0st_core::update`).

| File | Holds |
|---|---|
| `/var/lib/gh0st/update/state` | last check, result, the packages, download size, restart pending, last install |
| `/var/lib/gh0st/update/pending` | a check is owed: it was offline or metered |
| `/var/lib/gh0st/update/migrations` | the migrations done, one a line |
| `/run/gh0st-update/progress` | phase, percent, bytes, package, n of m. Rewritten at most 4 times a second, in tmpfs |
| `/run/gh0st-update/lock` | the engine's flock |

A tmpfiles.d entry makes the three directories at boot, so the inotify
watches always have a target.

**Units.** In `packaging/systemd/update`, installed by the gh0st package:

- `gh0st-update-check.service`: a check by hand. `gh0st-update-daily.service`:
  the daily one, which skips a metered network. Both oneshot, `Nice=10`,
  idle IO.
- `gh0st-update-install.service`: `Nice=5`. `KillMode=mixed`, so SIGTERM
  reaches only the engine, and shutdown waits up to 30 minutes for dpkg.
- `gh0st-update-daily.timer`: daily, `Persistent=true`, up to an hour of
  random delay, coalesced to the hour. It is the image's only enabled timer,
  and turning daily checks off disables it.
- NetworkManager dispatcher `90-gh0st-update`: on `up`, a pending check
  starts. A day without a network catches up when one joins, without
  polling.
- tmpfiles.d `gh0st-update.conf` makes the folders at boot. The package's
  postinst reloads systemd and makes them at once.

**Helper verbs.**

| Verb | Password | Does |
|---|---|---|
| `update-check` | no | starts a check |
| `update-install` | yes | starts an install |
| `update-stop` | no | stops a check, or a download |
| `update-auto on\|off` | yes | enables or disables the timer |
| `update-channel stable\|edge` | yes | adds or removes the edge source, then checks |
| `apt-clean` | no | `apt-get clean` |
| `store-refresh` | no | the Store's engine fetches the app list (`docs/store.md`) |
| `store-install`, `store-remove`, `store-upgrade` | yes | the Store's engine installs, removes or updates apps |

## The Updates app

`gh0st-update`, an SDK app. Its home tile is Updates.

| State | Shows | Enter | Chips |
|---|---|---|---|
| Never checked | Not checked yet | check | |
| Checking | a moving bar, the source | | 6 Stop |
| Up to date | the version, when it last checked | check | |
| Ready | the count and size, then a row each for Gh0st OS, Security and System (one line each when their notes don't fit) | install | 5 Check, 7 List |
| Downloading | a bar, MB of MB, the steps | | 6 Stop |
| Installing | a bar, the package, n of m | | |
| Done | what changed | restart | |
| Can't | why, and the fix | try again | 5 Check, 7 List while updates wait |

- Install asks for the password, then shows a card with the count and the
  size. It says when the device restarts at the end.
- With the kernel set in the list and the battery at 50% or below, Install is
  not offered. The screen says why and how much charge there is.
- Restart reboots the device when the kernel set, libc or systemd changed. When
  only Gh0st changed it restarts Gh0st, which closes open apps and locks the
  screen; the screen says so first.
- 7 List shows the packages by group, each with its versions and size. Enter
  opens one.
- The menu: Install all, Check now, List packages, Clear downloads, then Help.

It runs no timer while idle. The moving bar ticks only while it is on screen.

## Settings

System > Updates:

- The status row: "12 updates ready", or "Up to date". Enter opens the app.
- Check daily: on or off.
- Channel: Stable or Edge. Edge asks first: every build of main, and things
  may break. Going back to Stable keeps the edge build until Stable passes
  it. Nothing downgrades.
- Last check: the time, or what went wrong.

Each setting has one home. Check now and Clear downloads are in the app's
menu only; the daily check and the channel are in Settings only.

## Notifications

A file per notice. Root services, our apps and stock apps can all post one,
without a daemon.

- `/var/lib/gh0st/notify/` is for system services, `~/.local/state/gh0st/notify/`
  for apps.
- The file is named `<app>.<tag>`. Posting the same tag replaces the old one.
- Lines: `app`, `title`, `body`, `tone`, `time`, `open` (the desktop id
  Enter opens), and `count` (a number for the app's tile).

The shell watches both folders with inotify. A new notice:

- shows as a toast over whatever is in front, with the app's icon and name;
- waits while the screen is dark or locked. The lock screen shows only how
  many there are, never what they say;
- lights a bell and a count in the top bar on home, the switcher and the
  panel;
- puts a count on the app's tile.

The panel opens from 5 in the switcher and from the home menu. Enter opens
the notice's app. 8 Clear clears one. Holding 4 clears all, as in the
switcher.

## Our apt repo

`https://therezor.github.io/CardPuterZeroOS/apt`, suites `stable` and
`edge`, component `main`.

It holds our packages, the Raspberry Pi debs `lock.json` pins for the image,
and the stock app debs.

- **One pool.** A file in the pool is never replaced. Versions come from the
  commit's time and hash, and the kernel set keeps its version when its
  modules are rebuilt. So two builds can make different bytes under one
  name. The first one stays.
- **Stable is promoted.** A tag republishes what edge built for that commit,
  so both suites point at the same bytes.
- **Two versions kept** of each package per suite. A device with an index
  from before a publish still finds its files.
- **By hash.** `Acquire-By-Hash: yes`, so the Pages cache can't serve a
  Release and a Packages that don't match.
- **Valid-Until**, 30 days. A daily job signs again, so a frozen copy of
  the repo can't hold devices back.
- **The key.** An Ed25519 primary kept offline, and a signing subkey in the
  `APT_SIGNING_KEY` Actions secret. The image ships the public key in
  `/usr/share/keyrings/gh0st-archive-keyring.pgp` and prefers our origin
  (`Pin: release o=Gh0st`, 990).
- **Limits.** Pages takes 1 GB per site. CI fails the publish past 800 MB.
  Past that, the pool moves to release assets.
- **Versions move with their sources.** `os/build.sh` stamps each package
  with the last commit that touched it: gh0st and gh0st-term with the code,
  gh0st-base with its own folder and `os/repo`, the kernel set with
  `os/kernel`, its packages and the firmware pins in `lock.json` (the kernel
  and M5 commit are in its version already). A docs-only push publishes
  nothing new.
- **CI.** `.github/workflows/apt.yml` builds the debs, restores the pool from
  the Actions cache (falling back to the live site), publishes, builds the
  Store's catalog (`docs/store.md`), and deploys Pages. A tag also attaches
  its debs to the release. Setup of the key and
  Pages: `os/repo/README.md`.
- **The image.** `gh0st-base` ships `os/repo/gh0st.sources`, the pin
  and the key once `os/repo/gh0st-archive-keyring.pgp` exists; before
  that the image has no Gh0st OS source, since a source apt can't check
  would fail every check.

## Tested

The spike on 2026-11-22 ran in a container made from the release image's
own root tree (`rootfs.tar`), with a throwaway Ed25519 key:

- The image's apt (sqv) checks the Ed25519 signature.
- gh0st upgrades itself. `/usr/libexec/gh0st-helper` stays a hard link to the
  new `/usr/bin/gh0st`. No conffile asks a question, and `dpkg --audit` is clean.
- A rebuilt kernel set with the same version keeps the bytes already in the
  pool.
- A new kernel set moves as one. A half-published set (the meta package
  without its modules) is held back whole, so nothing breaks.
- The first `apt-get update` fetches 10.2 MB, nearly all of it Debian's main
  Packages. Later ones fetch diffs.
- `apt-get update`'s Status-Fd percent lags far behind, then jumps. The
  engine takes the larger of it and "Retrieving file N of M".
- The Updates app ran beside it under `--sim` and followed the engine
  over inotify, from the check through Download, Install and Cleaning up to
  "17 updates installed". `systemd-analyze verify` passes the units.
- The release image builds with all of it and passes its checks: 843 of
  850 MB, the daily check's the only timer.
- The engine itself (`gh0st update check`, then `install`) ran there as
  root: 14 updates found in 4 s, the notice posted, the download and dpkg
  phases reported, the cache cleaned, and "Restart to finish" posted for
  libc6.

The captured output is the engine's parser fixtures, in
`crates/gh0st/src/update/testdata`. The device test is still to come.
`apt-get update --error-on=any` makes a failed source fail the run, so an
offline or unsigned check never passes on old lists.
