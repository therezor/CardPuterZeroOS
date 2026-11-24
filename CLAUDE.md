# CardPuterZeroOS

**Gh0st OS**, a lean OS for the M5Stack CardputerZero. This workspace is its
Rust shell: the launcher, plus Settings and the terminal as apps. They
replace the stock C++/LVGL APPLaunch and keep its app contract. The distro
side is in `docs/distro-plan.md`.

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

- `crates/gh0st-ui`: RGB565 canvas, fonts and icons baked by `build.rs`, widgets,
  theme.
- `crates/gh0st-shell`: the home screen (grid or list, nav and search modes),
  the switcher and the dock. `examples/mockups.rs` renders its screens to PNG.
- `crates/gh0st-core`: syscalls, event loop, input, config, apps, children.
- `crates/gh0st-dbus`: our D-Bus codec and connection. SASL EXTERNAL, the
  wire format with every spec limit checked, fd passing, one poll loop and
  no threads. `tests/fuzz.rs` mutates real messages.
- `crates/gh0st-keyringd`: the keyring, `gh0st keyring`. The vault, the
  Secret Service and the shell link. `docs/keyring.md` is its guide.
- `crates/gh0st-keyring`: the Keyring app, an SDK app with its own binary.
  `examples/mockups.rs` renders its screens into `mockups/keyring` from the
  real Secret Service (`tests/common/fake.rs`).
- `crates/gh0st-session`: `gh0st session`, Notifications and ScreenSaver on
  gh0st's notices and `awake`.
- `crates/gh0st-app`: the app SDK. Pages, OS bars, soft keys, lists, jobs,
  a sim backend and a test driver. Apps depend on it alone. The guide is
  `docs/sdk.md`, which is also the crate's rustdoc, so its examples compile in
  `cargo test`.
- `crates/gh0st-settings`: Settings, an SDK app with its own binary. The page
  model is in `src/model`, the controller in `src/app.rs`.
  `examples/mockups.rs` renders every page into `mockups/settings`.
- `crates/gh0st-term`: the terminal, an SDK app with its own binary. `src/vt`
  is the emulator on `vte`; `docs/terminal.md` is its guide.
- `crates/gh0st-update`: Updates, an SDK app with its own binary. It shows
  what the update engine found and asks gh0st-helper to check or install.
  `examples/mockups.rs` renders its pages into `mockups/update-app`.
- `crates/gh0st-store`: the Store, an SDK app with its own binary. It lists
  CardputerZero apps from our signed mirror of M5's catalog and asks
  gh0st-helper to install them. `examples/mockups.rs` renders its screens
  into `mockups/store`.
- `crates/gh0st-camera`: Camera, an SDK app with its own binary. It drives
  `gh0st-camd` (`camd/`, C++ against libcamera, built in the image
  container) over a socket pair and a frame ring. `src/fake.rs` stands in
  for the helper on a host. `examples/mockups.rs` renders its screens into
  `mockups/camera`.
- `crates/gh0st`: one multi-call binary. The name it runs under picks the
  program. `gh0st update` is the update engine and `gh0st store` the
  Store's, both run as root by systemd.
- `os/store/mirror.py`: CI's mirror of M5's registry, the Store's signed
  catalog on our Pages (`os/build.sh store`).
- `assets/`: Inter, JetBrains Mono and Terminus (OFL; the Terminus BDFs are
  kept as published), Lucide icons (ISC), our mark, and the terminal's Nerd
  Font icons (a subset, `assets/fonts/nerd/README.md`).
- `docs/design.md`: the design system. Frame, colour roles, type, the top bar,
  chips over keys 5-8, keys, dialogs, feedback. `crates/gh0st-ui/examples/design.rs`
  renders its pictures.
- `docs/apps.md`: what the shell gives an app, and the shell link. Everything past
  home is an app built to it: Settings, the terminal.
- `docs/updates.md`: the update engine, the Updates app, notifications and
  our apt repo on GitHub Pages (`os/repo/publish.sh`).
- `docs/roadmap.md`: what comes next.
- `docs/testing.md`: the test layers, the design lint, and what a change
  must add.
- `docs/store.md`: the Store, its signed catalog, what the signature does
  not cover, the engine and the screens.
- `docs/setup.md`: first-boot setup, the steps and how it hands over to the
  shell.
- `docs/camera.md`: the camera, its quality ceiling, the helper protocol
  and the milestones.
- `docs/hotspot.md`: the Wi-Fi access point, why it takes wlan0, its security
  and its tests.
- `docs/keyring.md`: the keyring and session services, the vault, prompts,
  password changes and the limits.
- `docs/panel-driver.md`: how a frame reaches the panel, how the driver times
  sends against the scan, and the present ioctl that paces animations.
- `packaging/`: the systemd drop-in that swaps the stock launcher binary, and
  our `.desktop` entries. The `gh0st` package installs from here too.
- `os/`: the image build. `os/build.sh` runs it in a container, `os/lock.json`
  pins every input, `os/packages/` holds our debs. How: `docs/image-build.md`.
- `scripts/deploy.sh`, `scripts/measure.sh`: install on a device, measure it.
  `scripts/flash.sh`: write an image to an SD card.

## Rules

- **Performance first.** 512 MB, an SPI panel, a battery. No timer runs while
  idle unless it shows something (the clock ticks once a minute). Nothing polls
  in the background: use inotify, netlink, uevents, signalfd. The one system
  timer is the daily update check, which Settings turns off.
- **Draw cheap, flush less.** A frame redraws the 320x170 back buffer (about
  a millisecond); the flush then writes only the rows that changed, which is
  where SPI time goes. After an app or the boot splash, write every row.
- **No threads at idle.** One `poll` loop. Slow helpers are child processes
  whose pipes join the loop.
- **Geometry and colour live in `theme.rs`.** Widgets never hardcode either.
- **Keep the stock app contract.** `.desktop` files in
  `/usr/share/APPLaunch/applications`, cwd `/usr/share/APPLaunch`, the
  `LV_LINUX_*` and `APPLAUNCH_LINUX_*` env, ESC held 3 s sends SIGTERM to the
  group, SIGKILL 3 s later. Never `EVIOCGRAB` while an app is in front; the
  shell grabs only while it is in front with apps paused, dark, or locked.
- **Shared config.** `~/.config/cardputerzero/config.json` is read by stock
  apps too. Keep unknown keys. Our keys use a `gh0st.` prefix.
- New dependencies need a reason. Today: `libc`, `png`, `ab_glyph`,
  `qrcodegen` (the hotspot's QR code; no dependencies of its own), and in the
  terminal `vte` (the DEC parser, without std so its OSC buffer is bounded) and
  `unicode-width` (wcwidth). In the keyring: `argon2`, `chacha20poly1305`
  and `zeroize`, RustCrypto's, because crypto is never ours to write. Build
  only: `resvg`. The camera helper is C++
  because libcamera has no C API: it links libcamera and libjpeg-turbo, both
  on the stock image already.

## Build and test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release --target aarch64-unknown-linux-musl   # static, rust-lld
```

`.github/workflows/ci.yml` runs these and `cargo fmt --check` on every push
and PR. Core, the shell and the SDK deny `unwrap` outside tests: release
builds abort on a panic.

The image (Docker Desktop running; details in `docs/image-build.md`):

```sh
os/build.sh fetch && os/build.sh debs && os/build.sh image release && os/build.sh check release
```

Mockups. Both env vars are optional: without `GH0ST_STOCK` app tiles fall back
to letter badges, and without `GH0ST_CJK_FONT` CJK text draws as empty boxes.

```sh
GH0ST_STOCK=<launcher>/projects/APPLaunch/APPLaunch \
GH0ST_CJK_FONT=$GH0ST_STOCK/share/font/NotoSansCJK-Regular.ttc \
cargo run -p gh0st-shell --example mockups   # writes mockups/*.png
```

## Device loop

```sh
GH0ST_HOST=pi@<device> scripts/deploy.sh            # build, install, restart
GH0ST_HOST=pi@<device> scripts/deploy.sh rollback   # back to stock
GH0ST_HOST=pi@<device> scripts/measure.sh gh0st       # PSS, wakeups, CPU idle
GH0ST_HOST=pi@<device> scripts/tear-test.sh 10    # full-screen stripes at the panel rate
```

`GH0ST_LOG=debug` logs the first frame time and, for each frame, the rows flushed and
the wake-to-draw, draw and flush times. SDK apps log each wake and what caused it (`signal`,
`shell`, `keys`, `job`, `watch`, or none for a timer). The shell doesn't pass
`GH0ST_LOG` on: set it in a wrapper around the app's binary.

## Host runs

`gh0st shell --sim DIR`, and `--sim DIR` on any SDK app such as
`gh0st-settings`, runs the real loop on any host: keys come from a script
on stdin (format in `crates/gh0st-core/src/script.rs`), frames land in DIR as
PNGs. Point it at test apps and a scratch home so nothing real changes:

```sh
HOME=/tmp/gh0st-home GH0ST_APPS_OURS=... GH0ST_APPS_STOCK=... GH0ST_ICON_ROOT=... \
GH0ST_LOG=debug cargo run -p gh0st -- shell --sim /tmp/frames < session.txt
```

The shell starts on the lock screen. A session opens with `tab` then `enter`,
or sets `"gh0st.lock.password": false` in the scratch config.json.
`GH0ST_PASSWD` and `GH0ST_CHKPWD` swap `passwd -S` and `unix_chkpwd` for test
scripts.
