# App isolation

Every app the shell starts runs in a sandbox of its own and in a cgroup of
its own. The app is asked for nothing, so Store apps, stock apps and apps
installed by hand get it the same way ours do. The sandbox is bubblewrap
(`bwrap`), the launcher Flatpak uses. `crates/gh0st-core/src/sandbox.rs`
builds its command; the shell runs it from `launch`.

## What changes for an app

| | In a sandbox | Outside |
|---|---|---|
| User | UID 1000, its groups and devices | the same |
| Other processes | only its own: it can't trace, signal or read another app | all of UID 1000 |
| Home | read-write, but start files, `~/bin`, `~/.local/bin`, user units, D-Bus, PipeWire and WirePlumber config, and our other apps' data are read-only | read-write |
| Secrets | `~/.ssh`, `~/.gnupg` and the keyring's vault folder are empty; `~/.bash_history` can't be read | readable |
| Runtime folder | only the session bus, PipeWire and pulse sockets | everything, systemd's private socket too |
| cgroups | `/sys/fs/cgroup` is empty: it can't leave its own | it can move |
| setuid | sudo and pkexec raise nothing | they work |

What the user can take away, in Settings > Apps > the app. Each counts
from the app's next start, and every app has all three until then.

| Switch | Off |
|---|---|
| Network | its own network namespace, loopback only, and no system bus: NetworkManager and BlueZ can't reach out for it either |
| Camera | the sensor, unicam, the ISP and their media devices are `/dev/null`; the codec Media plays through stays |
| Microphone | ALSA's capture nodes are gone; pulse comes through `pulse/native-nomic`, whose clients see no source and can't record what the device plays; no PipeWire socket |
| Isolation | none of the above: the app runs as any program of UID 1000 does |

## Who runs outside a sandbox

- Ours that need the whole home or pkexec: Settings, Terminal, Files, Text,
  Store, Updates, Keyboard and Keyring (`X-Gh0st-Sandbox=false`). The key
  counts only in our own app folder. Camera and Media run sandboxed: they
  read files from anywhere.
- An app whose `Exec` runs `sudo`, `pkexec`, `doas`, `su` or `run0`.
- A Store app the catalog marks `root`: CI found a setuid file, or sudo,
  pkexec or doas as a command in its code (`store.md`). The Store asks
  before it installs one.
- An app the user turned Isolation off for.

The two automatic ones come from the app itself, so a hostile app can opt
out. Settings > Apps shows why an app runs outside ("Built in" or "Runs as
root").

## The home folder

Stock apps write anywhere in home, so it stays writable. What a program
outside a sandbox later runs or loads is locked: an app that could write
`~/.bash_profile` or `~/.local/bin/ls` would leave its sandbox the next
time the terminal starts. A path a sandbox locks must exist, or the app
could make it first, so the shell makes these at start where missing:

- `~/.profile`, `~/.bashrc`, `~/.bash_logout`: empty, which reads the same
  as none.
- `~/.bash_profile`: it sources `~/.profile`, as bash does without one.
  Skipped when `~/.bash_login` exists.
- `~/.inputrc`: `$include /etc/inputrc`.
- `~/bin`, `~/.ssh`, `~/.gnupg`, and the locked folders under `~/.config`,
  `~/.local/share`, `~/.local/state` and `~/.local/lib`.
- Our apps' data and cache folders, `gh0st-<app>`. A sandboxed app writes
  only its own.

## Identity

Each app runs in `app-<desktop stem>` right under the shell's cgroup
(`Delegate=yes` on `gh0st.service`). `gh0st_core::ident` reads it, so the
keyring and the session name a caller by its app. Our own programs keep
what they stored or were allowed before, under their old name
`exe:/usr/bin/gh0st-<app>`.

A sandboxed app posts notices into `notify/<stem>/`, bound over the shared
notice folder. The shell names such a notice by its folder, whatever the
file says, and Enter opens only that app.

Pulse reports a pid inside the sandbox, so the shell sets
`PULSE_PROP=gh0st.app=<stem>` and `gh0st audio` parks a paused app's
streams by that.

## The session bus

Every app is UID 1000, and the session bus lets any of them ask systemd to
start a process, which would run outside every sandbox.
`/usr/share/dbus-1/session.d/gh0st.conf` refuses, for everyone:

- anything sent to `org.freedesktop.systemd1`;
- `UpdateActivationEnvironment`, which would put `LD_PRELOAD` in every
  service started after;
- `BecomeMonitor` and eavesdropping, which would show every app's messages,
  the keyring's replies too.

`systemctl --user` keeps working through systemd's private socket. So does
`systemd-run --user`, except with `--wait`, `--pipe` or `--pty`, which take
the bus. `dbus-monitor` doesn't work.

## How it starts

The shell runs `bwrap` with its own user, mount, PID and IPC namespaces and
no new user namespaces inside. The app's process group, `LV_LINUX_*` env and
shell link pass through, so pause, resume and Esc held work as before. bwrap
adds two small processes, about 0.4 MB PSS.

- No `--die-with-parent`. With it, the shell's SIGTERM kills bwrap at once
  and bwrap the app, before the app's own handler can save.
- Grants are read at each start, not from the shell's copy of the config.
- No home, no runtime folder, or no `bwrap`: the app doesn't start, rather
  than start outside its sandbox.

The switches and the security keys (the terminal command, the lock, the
Store's mirror, recent files, the clipboard) live in
`~/.config/gh0st/settings.json`, which a sandbox can read but not write.
Stock apps write `config.json`, so a value one writes there for these keys
is ignored. The shell moves them out of `config.json` once.

A new `gh0st-base` asks for a restart of the device: its bus policy and its
audio rules hold only from a fresh session.

## Limits

- All apps are one user. An app outside a sandbox can do what UID 1000 can,
  including moving into another app's cgroup, which the keyring then takes
  for that app.
- An app granted the network keeps the system bus: NetworkManager and BlueZ
  answer it as they answer any program of the device user.
- The kernel has no Landlock and no Yama, so nothing narrows an app outside
  a sandbox.
- Config files of programs installed later, a `.zshrc` say, are locked only
  once they exist.
- An app that needs a microphone to start may wait forever with the
  microphone off. Recorder does.
- An app that asks for root at run time without saying so in its `Exec` or
  its code fails in a sandbox. If it quits within 5 s, the shell points at
  Settings > Apps.

## Tests

- `sandbox.rs` unit tests: the bwrap command for each switch and lock.
- `crates/gh0st-core/tests/sandbox_run.rs`: a real bwrap, where it and user
  namespaces work (Linux CI, the device). An app keeps Documents, new
  folders and its notices, and reaches no secret, start file, other app's
  data, process or network.
- On the device: the escapes above, each refused; Camera photo and video
  with sound, Media, a `Terminal=true` app, and Store apps run plain and
  sandboxed under `strace -f -Z` with no new failures.
