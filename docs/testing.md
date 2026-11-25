# Testing

What keeps bugs out, the look the same and the device fast. CI
(`.github/workflows/ci.yml`) runs every layer but the device on a push to
main or a PR that touches the code, assets, docs or `os/store`. It runs on
the same arm64 runner the image builds on, and also makes the static
device build. The device layer is run by hand.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## The layers

| Layer | Where | What it holds |
|---|---|---|
| Unit tests | next to the code | parsers, models, the config file, the lock field |
| Driver tests | `crates/*/tests`, `gh0st_app::testing::Driver` | a page's keys, dialogs, toasts and jobs, in virtual time |
| Sims | `crates/gh0st/tests/lock_sim.rs`, `crates/gh0st-settings/tests/*_sim.rs`, `forget.rs` | the real binary or a page with fake system tools first on `PATH` |
| Design lint | `gh0st_ui::lint`, the mockups, `crates/gh0st-settings/tests/pages.rs` | every screen in every palette keeps design.md's rules |
| Source rules | `crates/gh0st-ui/tests/rules.rs` | colours and faces only from the theme; no em dash |
| Idle budget | `Driver::idle_wakes`, `crates/gh0st-settings/tests/pages.rs` | an idle page wakes for the clock and nothing else |
| Store mirror | `os/store/test_mirror.py` | CI's mirror of M5's catalog, from a fixture registry |
| Sandbox | `crates/gh0st-core/tests/sandbox_run.rs` | an app in a real bwrap keeps its files and reaches no secret, start file, process or network; runs where bwrap and user namespaces work (CI sets the sysctl Ubuntu's AppArmor needs) |
| Compat suites | `crates/gh0st-keyringd/tests/compat.sh`, `crates/gh0st-session/tests/compat.sh`, `crates/gh0st/tests/keyring_sim.sh` | real D-Bus clients against our session services on a private bus, and the shell with both daemons |
| Device | `scripts/measure.sh` | PSS, wakeups and CPU at idle, before a release |

## On a device

The suite runs on the device too, where the Linux-only paths are real. Build
it with a target dir the device can mirror, since the sims find the app
binaries by the path they were built at:

```sh
CARGO_TARGET_DIR=/tmp/gh0st-xtarget cargo build --workspace --bins --target aarch64-unknown-linux-musl
CARGO_TARGET_DIR=/tmp/gh0st-xtarget cargo test --workspace --no-run --target aarch64-unknown-linux-musl
```

Copy the test binaries anywhere, and the app binaries to that same debug
path (`/tmp` there is a 32 MB tmpfs, so link it to a folder in home). Run
each with `TMPDIR` and `HOME` in scratch folders under home. No root is
needed, and the tests stub every system tool they touch.

## The rules

- **A fix comes with the test that failed before it.** Prove a bug from an
  audit or a report this way before fixing it.
- **A new page joins the lint.** A Settings page goes in `pages()` in
  `crates/gh0st-settings/tests/pages.rs`. Any other screen gets a scene in
  its crate's `examples/mockups.rs`. Those examples are tests (`test = true`
  in `Cargo.toml`): `cargo test` draws every scene in every palette under
  the lint, and `cargo run --example mockups` writes the same scenes as PNGs
  to look at.
- **No golden images.** The lint holds the rules a picture would; a change
  of look is reviewed by eye from the mockups.
- **Shared keys have one reader.** A key more than one program reads is
  named in `gh0st_core::config::key`, with its default in one place.
- **Each setting has one home.** `no_setting_is_on_two_pages` fails if an
  action shows up on two Settings pages.

## The design lint

`gh0st_ui::lint` checks what is drawn while a test turns it on. Release
builds keep none of it. `Driver::lint()` draws a page's frame under it.

- Text whose ink leaves its box by more than a descender's 2 px.
- A page's text under the bars: the runtime tells the lint the page's area
  while it draws.
- A face under the 15 px floor, but for Mono's figures (design.md, Type).
- Chips off their keys: one goes on 6, two on 5 and 7, three on 5 to 7.

## What is not covered

- Rows flushed per frame: `GH0ST_LOG=debug` prints them on a device or
  under `--sim`.
- The terminal's grid: `crates/gh0st-term/tests/golden.rs` replays recorded
  programs as text.
