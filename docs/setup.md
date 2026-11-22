# First-boot setup

A freshly flashed card has no user. Setup runs once, on the device, and makes it the
user's: who they are, where they are, how private the device is, and how it looks. Then
it hands over to the shell. This is roadmap M4's first part. The image side is in
`image-build.md`.

Status: designed 2026-11-21, moved onto the design system (`design.md`) the same day.
Mockups in `mockups/setup` (`cargo run -p gh0st-settings --example setup_mockups`).

## What the user decided

- The account name is chosen at setup. The release image ships with no user.
- A password is required. Settings has **Unlock with password** (on by default), so the
  lock screen can skip it. Settings' protected actions, sudo and SSH always ask for it.
- Setup's privacy review and Settings > Privacy are the same rows, calling the same
  helper verbs. Every privacy default the image ships can be switched in both.

## The steps

320x170: the top bar, 132 px of content, the action bar. One idea per screen and at most
four lines of body text.

First, while builds are pre-alpha, a notice (user, 2026-11-21): "A pre-alpha test build.
It can be unstable, and the SDK and the design can still change." Enter goes on.
`tour::PRE_ALPHA` drops it.

After the welcome, setup comes in two parts: a **training** of the keys and the design
system, then the **settings** (user, 2026-11-21). The welcome screen offers both: Enter
starts the training, and 6 skips straight to the settings. Each part counts its own steps.

Setup is drawn and driven like every other screen (`design.md`), so the user meets the
real thing from the first key:

- The top bar names the step and counts it within its part: "Moving  Training 3 of 9",
  "Account  Setup 6 of 8". The welcome screen has no count.
- **Enter goes on, Esc goes back.** No chip repeats them. Esc on the welcome screen does
  nothing.
- A step's own actions are chips over 5 to 7, spread by the design rule: Skip, Scan. Where
  Enter already works the rows (a switch, a choice), a main chip, Done, on 6 goes on.
- 8 opens setup's menu: Power off and Help, and during the training Skip training first.
  There is no Close app. Skip on a lesson goes to the next one.
- While a field takes text the bar turns into key hints: "Esc back  Tab next field
  Enter next".
- Fn+H lists every key on the step, as everywhere.

**Welcome.** The mark, "Welcome to Gh0st OS", and "We know nothing about you. Let's keep
it that way." Key pills name both ways on, "Enter take the training" and "6 I'm already a
spy", and 6 has a Skip chip. Esc goes back to the notice.

**The settings**, "Setup 1 of 8" to "Setup 8 of 8":

| # | Step | What it does |
|---|---|---|
| 1 | Country | The `iso3166.tab` list; a letter jumps to it, as in Settings. Sets the Wi-Fi country (`regdom`) and narrows the time zones. 7 Skip keeps the world country. |
| 2 | Time zone | The zones `zone.tab` gives that country, as far as `timedatectl list-timezones` knows them. "Other" opens the region list. 7 Skip keeps the zone. |
| 3 | Privacy | The private defaults, each switchable: Wi-Fi address per network, random address while scanning, hostname not sent, IPv6 privacy addresses, network time, logs off, SSH off, Bluetooth off. Enter flips a row; Done on 6 goes on. It comes before the network, because the address and hostname choices apply when a network is joined. |
| 4 | Network | The Wi-Fi list: Enter joins, 5 Scan, 7 Skip. It unblocks the radio first: the image boots with it blocked. With a cable in, "Use the cable" makes the wired profile (not built yet). Nothing goes online before this step. |
| 5 | Date and time | Only when the clock has not synced. The date and the time as two fields, filled in, to correct. Saved to the system clock and the RTC. |
| 6 | Account | The name and the password, then the password again on its own screen. |
| 7 | Device name and look | The device name (`zero` unless changed), the home layout, the palette (Mint, Nord, Paper) and the clock. 5 and 7 change a choice; Done on 6 goes on. The look applies at once. |
| 8 | Done | What was set, then "Starting" while the shell comes up. |

Esc on Country goes back to the training's last screen, or to the welcome screen when the
training was skipped.

### The training

"Training 1 of 9" to "Training 9 of 9": the keyboard first, then the design system, by
using it, as agent training with a short joke a screen (user, 2026-11-22). One idea each.
Skip on a lesson goes to the next; Skip training, in the menu, goes straight to the
settings. A wrong letter says "Typo. The van outside saw that."

| # | Screen | On screen | Passes when, and what it says |
|---|---|---|---|
| 1 | Typing | "Raise your right hand. Promise you won't hack anything. Type opsec to swear." | `opsec`: "Sworn in. We believe you. Mostly." |
| 2 | Caps lock | "Now say it like you mean it. Press Shift twice for caps lock." | `OPSEC`: "The neighbours heard. Fine." |
| 3 | Moving | "Field sobriety test. Walk the dot to the ring. Straight line, agent." f x z c move. | Enter on the ring: in 3 moves "Sober. Suspiciously sober.", else "Zigzag. We'll call it evasive driving." |
| 4 | Symbols | "Give us your email. Kidding. Type a fake one." Sym gives the @. | `nobody@nowhere.net`: "Perfect. We'll never write." |
| 5 | The bars | Callouts to the top bar. "See 7? It says Don't. So don't." | the Don't chip: "You pressed it. Of course you did." |
| 6 | Rows | "Put on your tinfoil hat. Turn paranoia up." A switch, a choice, a page. | hat on, Paranoia at Max: "Fashionable and unreadable." |
| 7 | Menu and help | "Secret menu on 8. Find Next lesson. Tell no one. Fn and H list every key." | Next lesson from the menu: "It was never secret. Act surprised." |
| 8 | Taking care | "Hold 7 to self-destruct. Relax, it's a drill." Erasing wants a hold; so does leaving an app, Esc; Fn+K switches. | the red Boom chip held: "Boom. Nothing happened. Breathe." |
| 9 | Graduation | "Training complete. You were never here. Unlocked: Phantom, a palette for ghosts." | Enter wears it, 6 Later keeps the look |

**The reward.** Reaching Graduation sets `gh0st.tour_done`, which setup's config carries
into the user's. Phantom, near-black with a pale violet accent, is a built-in palette
(`theme::EARNED`) that Settings > Appearance lists only once it is set.

Screens 5 to 8 use the real chips, rows, menu and help, so what the user tried there is
what every app does. Holding Esc and Fn+K are described, not tried: no app runs yet.

Rules for the account step:

- The name matches `^[a-z][-a-z0-9_]{0,31}$` and is not an existing user or group:
  `useradd -U` fails on a name like `video`.
- The password has at least 4 characters. Tab moves between the two fields. On the
  "Again" screen Tab shows what was typed. A field that is wrong gets a red edge and says
  why under it.

## How it runs

**The unit.** `gh0st-setup.service`, a system unit run as root, ships in the `gh0st`
package:

```ini
[Unit]
Description=First-boot setup
ConditionPathExists=/var/lib/cpzeroos/setup
RequiresMountsFor=/home
After=cpzeroos-grow-data.service fb_load.service dbus.service polkit.service
After=NetworkManager.service systemd-timesyncd.service
Wants=NetworkManager.service
StartLimitIntervalSec=0

[Service]
Environment=HOME=/var/lib/cpzeroos/setup-home
WorkingDirectory=/usr/share/APPLaunch
ExecStart=/usr/bin/gh0st-settings --setup
Restart=always
RestartSec=1

[Install]
WantedBy=multi-user.target
```

**Why root.** Stock's wizard runs as root before any user session, for the same reason:
there is no user yet. `nmcli` and `timedatectl` need no polkit as root, which also holds
once NetworkManager gets `auth-polkit=root-only` (roadmap M2).

**The program.** `gh0st-settings --setup`. The same binary as Settings, so setup reuses
its Wi-Fi, country, time zone, appearance and logs code without making it public. Root
actions keep Settings' call, `pkexec /usr/libexec/gh0st-helper ...`: the helper already
passes a root caller without a password.

**No way out but finishing.** The SDK app runs without "Close app" in its menu and without
hold Esc to exit, and the first step ignores Esc. SIGTERM ends it, and the unit starts it
again.

**Settings for the look.** Setup's `HOME` is `/var/lib/cpzeroos/setup-home`, so the look
choices land in its own `config.json` through `Cx::settings()`, and theme setup at once.
`user-create` installs that file into the new home.

**No shell runs during setup.** Setup lights the panel at start when it is dark. Fn+K and
holding Esc do nothing; the training only describes them. Not built yet: the brightness keys,
and powering off after 10 idle minutes on battery.

## Power cuts

**Resume.** `/var/lib/cpzeroos/setup.state` (root, 0600) keeps the step, whether the
training was taken, and the account name. Never the password, so a resume never starts past
the account step. System steps apply as they are made and can run twice. The account is
made only on Done. When a cut came after `user-create`, the account step keeps that name
and asks only its password. When `setup-finish` fails, Done offers Restart (`power
reboot`): the marker may be gone already, and either boot path is then right.

**Hand-over.** At any power cut, the next boot runs exactly one of setup or the shell:

- `gh0st.service` (the shell's user unit) has `ConditionPathExists=!/var/lib/cpzeroos/setup`
  and `ConditionUser=1000`.
- `setup-finish` writes the linger file and syncs it, removes the marker and syncs again,
  then starts `user@1000.service`.

A cut before the marker goes runs setup again with the user already made; `user-create`
finishes what is missing. A cut after it starts the shell.

## Helper verbs

The password always goes on stdin, never in the arguments. Setup runs as root, so the
gated verbs don't ask it. Each verb has its arm in `Verb::gated`, and parser tests. Built
2026-11-21 in `crates/gh0st/src/helper.rs`, the text in `gh0st_core::sysconf`.

| Verb | Stdin | Allowed |
|---|---|---|
| `user-create NAME` | the new password | Only during setup: the helper itself checks that the marker exists and the caller is root (exit 13 otherwise) |
| `setup-finish` | none | Only during setup |
| `set-time EPOCH` | the user's password | Gated. Sets the clock (`clock_settime`) and the RTC (`RTC_SET_TIME`), 2024 to 2099. `timedatectl set-time` is refused while network time is on. |
| `ntp on\|off` | the user's password | Gated. `timedatectl set-ntp` |
| `mac random\|stable\|device` | the user's password | Gated. `90-gh0st-mac.conf` |
| `scan-mac random\|device` | the user's password | Gated. `90-gh0st-scan-mac.conf` |
| `dhcp-hostname on\|off` | the user's password | Gated. `90-gh0st-dhcp-hostname.conf` |
| `ip6-privacy on\|off` | the user's password | Gated. `90-gh0st-ip6-privacy.conf` |

The network verbs each write one whole file in `/etc/NetworkManager/conf.d`, which
NetworkManager reads after the image's `50-cpzeroos.conf`, then reload it. A new
connection takes the change.

`user-create` runs `useradd -m -u 1000 -U -s /bin/bash` with the groups `sudo`,
`netdev`, `video`, `input`, `render`, `audio`, `gpio`, `i2c`, `spi`, `dialout` and
`plugdev`, leaving out any the system lacks. It sets the password with `chpasswd`,
installs the staged `config.json` (and, on a dev image, the staged SSH key), and can run
twice: UID 1000 with the same name carries on, any other clash refuses. `setup-finish`
writes the linger file and syncs it and its directory, removes the marker and syncs
`/var/lib/cpzeroos`, then starts `user@1000.service`. The existing verbs stay as they are:
`regdom`, `hostname`, `logs`, `ssh`, `rfkill`, `power`.

## What gets built

**SDK** (`crates/gh0st-app`), built 2026-11-21 with tests; the guide is `sdk.md`. The
chips, hint bar, menu, help and dialogs came with the design system.

- `Steps`: one page per step, each made as it opens with its label ("Setup 6 of 8").
  `part` numbers a part on its own.
  A step calls `cx.done()` to go on; Esc goes back. A maker that returns `None` is
  skipped. `from` resumes at a step, `on_step` hears each one.
- `run_with(page, Options { closable: false })`: no hold Esc to exit, no Close app, the
  last page stays. `cx.quit()` and SIGTERM still end it. Setup's `HOME` gives it its own
  config, so no path option is needed.
- `Form`: labelled fields, Tab between them, an error line under one, Tab shows a lone
  password. Zeroed when dropped.
- `testing::Driver::jobs` runs jobs to the end, so tests see their lines and codes.

**Settings** (`crates/gh0st-settings`), built 2026-11-21:

- `Settings::embedded(Embed { page, label, skip, done })`: one page as a setup step. It
  asks only what that page shows (no volume: no PipeWire as root), carries the step's
  label, puts Skip on 7 or Done on 6, and has Power off in its menu. Esc on its own page
  goes back a step. It calls `cx.done()` once a country or time zone is set, or a network
  joined, after the job's success. Tests in `tests/embed.rs`.
- `pages::privacy`: the eight privacy rows, for Settings > Privacy and setup alike. The
  network rows read what NetworkManager runs with from its config files
  (`sysconf::nm_privacy`) and change through the helper verbs.
- System > Date and time: a dialog, "2026-11-21 14:32", set through `set-time`.
- System > **Unlock with password**, config key `gh0st.lock.password`, default true.
- `PageId::Look`, setup's device name and look.

**The setup module** (`src/setup`), built 2026-11-21: `gh0st-settings --setup` runs
`setup::steps` under `run_with(.., Options { closable: false })`. The notice, the welcome
and the training are in `tour.rs`; Country, Time zone (`PageId::CountryZones`),
Privacy, Network and Device and look are embedded Settings; `clock.rs`, `account.rs` and
`finish.rs` are the rest. Done draws "Starting" before `setup-finish` runs, and quits when
it succeeds. `$GH0ST_SETUP_ROOT` moves the files setup reads and writes, for a host run.

**Shell** (`crates/gh0st-shell`): the lock screen asks for the password when
`gh0st.lock.password` is true and `passwd -S` says the account has one. It checks with
`/usr/sbin/unix_chkpwd`, which lets a user check their own password, and waits longer after
each wrong try. It starts locked, and locks on wake after `gh0st.lock.after` seconds of dark
(System > Lock after, 5 minutes by default). `design.md` has the screen; `apps.md`, Pausing,
has what happens to the app in front.

**Image** (branch `image`):

- Release: no user and no linger; the marker; `gh0st-setup.service` enabled.
- Dev: the dev user is made as today, with no marker. `CPZ_DEV_SETUP=1` builds a dev image
  that runs setup instead, with the SSH key staged for `user-create`.
- NetworkManager gets `no-auto-default=*`, so a cable stays offline until the user picks
  it.
- `fb_load.service` becomes `Type=oneshot` with a 5 s timeout, so the logo is done before
  setup draws.

## Tests

- Built: `tests/setup_sim.rs` runs `gh0st-settings --setup --sim` from a key script, with
  fake `pkexec`, `nmcli` and `timedatectl` first on `PATH`. It checks every helper call,
  `user-create` before `setup-finish`, the password on stdin only, and none in
  `setup.state`. `tests/embed.rs` checks the embedded steps' chips and Esc.
  A second script takes the training, passes every screen by its keys, and wears
  Phantom. The setup mockups draw the notice, the welcome and the training from the
  real pages, through `testing::Driver`.
- In setup's steps, digits don't pick rows, the network step's cursor keeps off the
  Wi-Fi switch, and Done, Skip and Esc wait for a change in hand to finish: closing a
  step stops its jobs.
- Still to add: a Wi-Fi join, a bad name, passwords that differ, and resuming at each
  step.
- The image's checks: the marker, no UID 1000, the unit enabled; the QEMU smoke test sees
  setup start and no `user@1000`.
- On the device, from a fresh flash: setup after the logo; no traffic before step 6; the
  chosen Wi-Fi address policy on join; the shell as the new user with sound; a power cut at
  each step resumes; SSH with the new password once turned on; the lock screen with and
  without **Unlock with password**.

To find out on the device along the way: whether the RTC keeps time across boots;
whether enabling linger starts `user@1000` at once; whether the world Wi-Fi country is
enough for a first join on the 2.4 GHz-only radio; whether NetworkManager applies the new
`conf.d` file before the join.
