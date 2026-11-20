# CPZeroOS

A lean OS for the M5Stack CardputerZero. This workspace is its Rust shell:
the launcher and Settings. They replace the stock C++/LVGL APPLaunch
and keep its app contract. The terminal is a separate project, installed as an
app. The distro side is in `docs/distro-plan.md`.

## Writing style

Applies to chat replies, commit messages, docs and code comments.

- **No em dashes.** Not `—`, not `-` standing in for one. Use a full stop, a
  comma, a colon, or brackets.
- **Say it once.** No restating the request, no summary of what you just wrote.
- **Short words and short sentences.** Split anything past about 25 words.
- **Cut hedging.** Drop "just", "simply", "basically", "essentially", "probably".
- **Comments are minimal.** Say why, not what.

## Hardware

Raspberry Pi CM0 (4x Cortex-A53, 512 MB). ST7789V 320x170 RGB565 on SPI, driven
by DRM `panel-mipi-dbi-m` with fbdev emulation at `/dev/fb_lcd`. TCA8418
keyboard on evdev (`/dev/input/by-path/platform-3f804000.i2c-event`). BQ27220
battery in `/sys/class/power_supply`. Details and sources in
`docs/distro-plan.md`. Nothing here is measured on a device yet: mark what is, in
`docs/hardware-facts.md`.

## Layout

- `crates/cpz-ui`: RGB565 canvas, fonts and icons baked by `build.rs`, widgets,
  theme.
- `crates/cpz-shell`: the home screen (grid or list, nav and search modes).
  `examples/mockups.rs` renders every screen to PNG.
- `crates/cpz-core`: syscalls, event loop, input, config, apps, children.
- `crates/cpz`: one multi-call binary. The name it runs under picks the program.
- `assets/`: Inter and JetBrains Mono (OFL), Lucide icons (ISC).
- `docs/apps.md`: what the shell gives an app. The terminal is built to it.
- `packaging/`: the systemd drop-in that swaps the stock launcher binary, and
  our `.desktop` entries.
- `scripts/deploy.sh`, `scripts/measure.sh`: install on a device, measure it.

## Rules

- **Performance first.** 512 MB, an SPI panel, a battery. No timer runs while
  idle unless it shows something (the clock ticks once a minute). Nothing polls
  in the background: use inotify, netlink, uevents, signalfd.
- **Draw cheap, flush less.** A frame redraws the 320x170 back buffer (about
  a millisecond); the flush then writes only the rows that changed, which is
  where SPI time goes. After an app or the boot splash, write every row.
- **No threads at idle.** One `poll` loop. Slow helpers are child processes
  whose pipes join the loop.
- **Geometry and colour live in `theme.rs`.** Widgets never hardcode either.
- **Keep the stock app contract.** `.desktop` files in
  `/usr/share/APPLaunch/applications`, cwd `/usr/share/APPLaunch`, the
  `LV_LINUX_*` and `APPLAUNCH_LINUX_*` env, no `EVIOCGRAB`, ESC held 3 s sends
  SIGTERM to the group, SIGKILL 3 s later.
- **Shared config.** `~/.config/cardputerzero/config.json` is read by stock
  apps too. Keep unknown keys. Our keys use a `cpz.` prefix.
- New dependencies need a reason. Today: `libc`, `png`, `ab_glyph`. Build only:
  `resvg`.

## Build and test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release --target aarch64-unknown-linux-musl   # static, rust-lld
```

Mockups. Both env vars are optional: without `CPZ_STOCK` app tiles fall back
to letter badges, and without `CPZ_CJK_FONT` CJK text draws as empty boxes.

```sh
CPZ_STOCK=<launcher>/projects/APPLaunch/APPLaunch \
CPZ_CJK_FONT=$CPZ_STOCK/share/font/NotoSansCJK-Regular.ttc \
cargo run -p cpz-shell --example mockups   # writes mockups/*.png
```

## Device loop

```sh
CPZ_HOST=pi@<device> scripts/deploy.sh            # build, install, restart
CPZ_HOST=pi@<device> scripts/deploy.sh rollback   # back to stock
CPZ_HOST=pi@<device> scripts/measure.sh cpz       # PSS, wakeups, CPU idle
```

`CPZ_LOG=debug` logs the first frame time and the rows each flush writes.

## Host runs

`cpz shell --sim DIR` runs the real loop on any host: keys come from a script
on stdin (format in `crates/cpz-shell/src/sim.rs`), frames land in DIR as
PNGs. Point it at test apps and a scratch home so nothing real changes:

```sh
HOME=/tmp/cpz-home CPZ_APPS_OURS=... CPZ_APPS_STOCK=... CPZ_ICON_ROOT=... \
CPZ_LOG=debug cargo run -p cpz -- shell --sim /tmp/frames < session.txt
```
