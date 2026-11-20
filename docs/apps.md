# What the shell gives an app

The contract any app relies on, stock or ours. The terminal, a separate
project, is built against this page.

## Finding apps

- `.desktop` files in `/usr/share/APPLaunch/applications` (stock and Store
  installs) and `/usr/share/zer0/applications` (ours).
- Keys read: `Name`, `Exec`, `Icon`, `Comment`, `TryExec`, `Terminal`,
  `Sysplause`, `Hidden`, `NoDisplay`, `Type` (must be `Application`).
- `Exec` follows the stock rule: no `| & ; < > $ \` or backtick, and it starts
  with an absolute path to an executable or one of `bash python3 vim vi nano sh`.
- `Icon` is a PNG path, absolute or relative to `/usr/share/APPLaunch`, or
  `zer0:<name>` for a symbol built into zer0. A missing icon shows the first
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
  | `ZER0_SYSPLAUSE` | `1` when a terminal should wait for a key after its command exits |

- The keyboard is never grabbed: the app reads it directly.
- While an app runs the shell draws nothing, except a banner across the top
  while ESC is held, and the low-battery countdown.
- The shell handles brightness, volume, mute and screenshot keys meanwhile.

## Leaving

- Exit normally. The shell returns when the whole process group is gone, so
  background children keep the app open.
- Holding ESC: a hint at 0.5 s, SIGTERM to the group at 3 s, SIGKILL 3 s
  later. Apps should exit cleanly on SIGTERM.
- The shell repaints the whole panel on return.

## Terminal

- Command from `zer0.terminal` in config.json, default `/usr/bin/zer0-term -e`.
- A `Terminal=true` entry runs as `<terminal> /bin/sh -c "<Exec>"`.
- `ZER0_SYSPLAUSE=1` means: after the command exits, wait for a key, then exit.

## Config

`~/.config/cardputerzero/config.json`, shared with stock. The shell re-reads
it after every app exits, so changes made by Settings take effect then.
