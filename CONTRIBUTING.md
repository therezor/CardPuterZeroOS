# Contributing

Thanks for wanting to haunt this repo. 👻

## Pull requests go to `dev`

Branch off `dev` and open your PR against `dev`, not `main`. New work gets
tested on real devices from `dev` (the Edge channel and the test build
follow it). `main` only moves when a release ships.

## Speed and memory come first

The device has 512 MB, an SPI panel and a battery. Every change is judged
by what it costs there.

- **Low memory.** Measure what your change adds (`scripts/measure.sh`
  shows PSS). Megabytes need a reason. Kilobytes are welcome.
- **Fast.** No work on the hot path that could happen once at startup or
  at build time. Redraw only what changed.
- **Quiet at idle.** No timers, no polling, no threads at rest. Use
  inotify, netlink, uevents or signalfd. Add a thread that wakes every
  100 ms and the battery will find you.

A feature that makes the device slower is a bug with a nice name.

## Before you open one

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same three. It is less polite about it.

Then say how you tested. "Ran it on a CardputerZero" is great. "Only in
`--sim`" is fine. Guessing is not.

## What gets merged

- **One thing per PR.** Two fixes, two PRs.
- **Numbers for anything that touches speed or memory.** Before and after,
  from a device if you have one.
- **A reason for every new dependency.** `CLAUDE.md` lists the ones we
  have and why.
- **Pictures for UI changes.** A mockup PNG or a photo of the screen.
- **Stock apps keep working.** Anything that installs from M5's Store must
  still install and launch.

## Style

Short sentences. No em dashes. Commit subjects follow the log,
`Area: what changed`, for example `Files: archives open as folders`.
Comments say why, not what.

## Bugs and ideas

Open an issue with the Build from Settings > About.
[docs/beta.md](docs/beta.md#report-a-bug) says what else helps.

## License

What you send is MIT, like the rest of the Rust code.
