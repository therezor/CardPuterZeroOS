# What the shell gives an app

The contract any app relies on, stock or ours. The terminal, a separate
project, is built against this page.

## Finding apps

- `.desktop` files in `/usr/share/APPLaunch/applications` (stock and Store
  installs) and `/usr/share/gh0st/applications` (ours).
- Keys read: `Name`, `Exec`, `Icon`, `Comment`, `TryExec`, `Terminal`,
  `Sysplause`, `Hidden`, `NoDisplay`, `Type` (must be `Application`).
- `Exec` follows the stock rule: no `| & ; < > $ \` or backtick, and it starts
  with an absolute path to an executable or one of `bash python3 vim vi nano sh`.
- `Icon` is a PNG path, absolute or relative to `/usr/share/APPLaunch`, or
  `gh0st:<name>` for a symbol built into gh0st. A missing icon shows the first
  letter of the name (stock drops such an entry).
- `TryExec` names a program that must exist, or the entry is hidden.
- An app with `Terminal=true` shows only when a terminal is installed.

## Running

- Own process group. cwd `/usr/share/APPLaunch`. stdin is `/dev/null`;
  stdout and stderr go to the journal.
- Third-party `Exec` runs as `/bin/sh -c "<Exec>"`, as in stock. Ours run
  their argv directly.
- Environment:

  | Variable | Value |
  |---|---|
  | `LV_LINUX_FBDEV_DEVICE`, `APPLAUNCH_LINUX_FBDEV_DEVICE` | the panel, usually `/dev/fb_lcd` |
  | `LV_LINUX_KEYBOARD_DEVICE`, `APPLAUNCH_LINUX_KEYBOARD_DEVICE` | the keyboard's evdev node |
  | `APPLAUNCH_LINUX_KEYBOARD_MAP` | the symbol-layer map file |
  | `GH0ST_SYSPLAUSE` | `1` when a terminal should wait for a key after its command exits |
  | `CAMERA_APP_KEYBOARD_GRAB` | `false`: the stock camera grabs the keyboard otherwise |

- The keyboard is never grabbed while the app is in front: the app reads it
  directly. An app must not grab it either, or Fn+K, ESC held and the
  volume keys stop working.
- While an app is in front the shell draws only over it: a banner across the
  top while ESC is held or the battery is about to run out, and a toast for
  brightness, volume, mute and screenshot keys. When one goes, the shell puts
  back the app's pixels, except any the app drew meanwhile.
- The shell handles brightness, volume, mute and screenshot keys meanwhile.

## Pausing

- Fn+K (`KEY_HOME`) pauses the app in front and opens the switcher. The app
  sees Fn and Home pressed and released; the shell waits until every key is
  up, then sends SIGSTOP to the group.
- The shell keeps the app's last frame. Before it sends SIGCONT, it puts that
  frame back on the panel, so an app that redraws only what changed still
  looks right.
- While apps are paused and the shell is in front, the shell grabs the
  keyboard. Before an app resumes, the shell lets go and writes `MSC_SCAN`
  events to the device until every paused reader's queue overflows, so the
  app reads `SYN_DROPPED` instead of keys typed elsewhere. Ignore `EV_MSC`.
- Closing a paused app from the switcher sends SIGTERM and SIGCONT together,
  then SIGKILL 3 s later. So does a stopping shell.

## Leaving

- Exit normally. The shell returns when the whole process group is gone, so
  background children keep the app open.
- Holding ESC: a hint at 0.5 s, SIGTERM to the group at 3 s, SIGKILL 3 s
  later. Apps should exit cleanly on SIGTERM.
- The shell repaints the whole panel on return, and on pause.

## Terminal

- Command from `gh0st.terminal` in config.json, default `/usr/bin/gh0st-term -e`.
- A `Terminal=true` entry runs as `<terminal> /bin/sh -c "<Exec>"`.
- `GH0ST_SYSPLAUSE=1` means: after the command exits, wait for a key, then exit.

## Config

`~/.config/cardputerzero/config.json`, shared with stock. The shell re-reads
it after every app exits, so changes made by Settings take effect then.
