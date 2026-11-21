# CardPuterZeroOS

A lean OS for the M5Stack CardputerZero. Its shell is Gh0stdeck, a Rust
replacement for the stock C++/LVGL launcher (APPLaunch). Stock apps and the
Store keep working.

<img src="docs/screenshots/home.png" width="640" alt="Gh0stdeck home screen on the device">

## Measured on the device

One minute idle on the home screen, measured with `scripts/measure.sh` on a
CardputerZero (CM0, 512 MB):

| | Stock APPLaunch 0.6.105 | Gh0stdeck |
|---|---|---|
| Memory (PSS) | 9.6 MB | 1.3 MB |
| Threads | 14 | 1 |
| Wakeups | 35,026 | 24 |
| CPU time | 5,430 ms | 10 ms |

The binary is about 1 MB, static, with its fonts and icons built in.

## What it does

- Shows every installed app as a 4x2 grid or a list. Settings picks which.
- Moves with F, Z, X, C or the Fn arrows. S starts a search, P pins an app,
  Enter or a digit opens one.
- Runs apps the way stock does, from the same `.desktop` files and with the
  same environment. Holding Esc for 3 seconds closes a stuck app.
- Fn+K pauses the app in front and opens the switcher: Home, then every open
  app. A paused app is stopped, not left running, and comes back showing
  what it showed. Del closes one. Keys typed elsewhere never reach it.
- Shows the skull, a slot for each open app, the network, battery and clock
  in the top bar. It reads them from sysfs when the kernel reports a change,
  so nothing polls.
- Settings is an app, `gh0st-settings`, built on the app SDK: Wi-Fi,
  network, Bluetooth, display, sound, look, which apps show, time zone, SSH,
  password, logs, about. Root actions go through a small helper that takes
  four fixed verbs. Fn+K pauses it like any app, and Esc on its first page
  closes it. The shell reads config.json again whenever it takes the screen
  back, so home follows what Settings changed.
- Brightness, volume, mute and screenshot keys work everywhere and show a
  toast, over apps too.
- Turns the screen off after the stock `dark_time` and locks when Tab is held
  for 3 seconds.
- Counts down and powers off at 3% battery, even over a running app.

It also fixes the panel driver. The stock driver sends frames without waiting
for the panel's refresh, and the panel is mounted rotated, so every app tore.
`os/kernel/panel-mipi-dbi-m` tracks the TE signal on GPIO5 and starts each send
where it can't cross the panel's scan. It sends lines in the panel's own scan
order and runs the panel at 50.6 Hz. Home animations wait on a present ioctl and
lock to the refresh, at 50 fps even for full-screen redraws.

## Install

From a Mac or Linux machine with Rust and SSH access to the device:

```sh
GH0ST_HOST=rezor@cardputer.local scripts/deploy.sh            # install
GH0ST_HOST=rezor@cardputer.local scripts/deploy.sh rollback   # back to stock
```

The deploy script installs `/usr/bin/gh0st` next to the stock package. A
systemd drop-in makes the stock `APPLaunch.service` start Gh0stdeck, so
applaunch upgrades and the first-boot wizard keep working.

The panel driver builds on the device, against the running kernel:

```sh
cd os/kernel/panel-mipi-dbi-m && sudo ./install.sh             # then reboot
sudo ./install.sh uninstall                                   # M5's driver back
```

## Build

```sh
cargo test --workspace
cargo build --release --target aarch64-unknown-linux-musl     # static binary
```

The release build links with the `rust-lld` that ships with rustup, so it
needs no cross compiler. `cargo run -p gh0st -- shell --sim DIR` runs the
shell on any host: keys come from a script on stdin and frames land in DIR
as PNGs.

## Layout

- `crates/gh0st-ui`: RGB565 drawing, fonts and icons baked at build time,
  widgets, themes.
- `crates/gh0st-core`: config, `.desktop` parsing, keyboard map, the event
  loop and child processes.
- `crates/gh0st-shell`: the home screen and the shell process.
- `crates/gh0st-app`: the app SDK. The guide is `docs/sdk.md`.
- `crates/gh0st-settings`: Settings, an app built on the SDK.
- `crates/gh0st`: the `gh0st` binary.
- `os/kernel/panel-mipi-dbi-m`: the patched panel driver and its overlay.
- `docs/`: hardware facts, the panel driver, the app contract, the distro plan.

## Status

The shell and the switcher run on the device. Settings as a separate app is
built and runs in the sim; it is not yet tested on the device. An app can be
allowed to keep running in the background; while another app is in front it
still shares that app's keyboard and panel. Still to build: the terminal,
which lives in its own repository.

## Licenses

The Rust code is MIT. The panel driver is GPL-2.0, from M5Stack and Noralf
Trønnes. Inter and JetBrains Mono are under the SIL Open Font License, and
the Lucide icons under ISC.
