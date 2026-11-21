# What the shell gives an app

This page is the contract between the shell and every app, stock or ours.
Everything past the home screen is an app, Settings and the terminal included.
Rust apps get all of it through the SDK (`sdk.md`). Apps in other languages
use this page directly.

## Finding apps

- The shell reads `.desktop` files in `/usr/share/APPLaunch/applications`
  (stock and Store installs) and `/usr/share/gh0st/applications` (ours).
- It reads these keys: `Name`, `Exec`, `Icon`, `Comment`, `TryExec`,
  `Terminal`, `Sysplause`, `Hidden`, `NoDisplay`, `Type` (which must be
  `Application`), and our own `X-Gh0st-Bars` (see Shell link).
- `Exec` follows the stock rule. It contains no `| & ; < > $ \` or backtick,
  and it starts with an absolute path to an executable or with one of
  `bash python3 vim vi nano sh`.
- `Icon` is a PNG path, absolute or relative to `/usr/share/APPLaunch`, or
  `gh0st:<name>` for a symbol built into gh0st. Without an icon the shell
  shows the first letter of the name. Stock drops such an entry.
- `TryExec` names a program that must exist. If it doesn't, the shell hides
  the entry.
- An app with `Terminal=true` shows only when a terminal is installed.

## Running

- The app runs in its own process group, with cwd `/usr/share/APPLaunch`.
  Its stdin is `/dev/null`, and its stdout and stderr go to the journal.
- The shell runs a third-party `Exec` as `/bin/sh -c "<Exec>"`, as stock
  does. It runs ours from their argv, without a shell.
- The app gets this environment:

  | Variable | Value |
  |---|---|
  | `LV_LINUX_FBDEV_DEVICE`, `APPLAUNCH_LINUX_FBDEV_DEVICE` | The panel, usually `/dev/fb_lcd` |
  | `LV_LINUX_KEYBOARD_DEVICE`, `APPLAUNCH_LINUX_KEYBOARD_DEVICE` | The keyboard's evdev node |
  | `APPLAUNCH_LINUX_KEYBOARD_MAP` | The symbol-layer map file |
  | `GH0ST_SYSPLAUSE` | `1` when a terminal should wait for a key after its command exits |

- The shell never grabs the keyboard. The app reads it directly.
- The shell draws over an app only when the app asks for OS bars (see Shell
  link), when ESC is held (a banner across the top), and for the low-battery
  countdown.
- The shell acts on the brightness, volume, mute and screenshot keys while an
  app runs.

## Shell link

Status: specified, not built yet (roadmap A1). Until it lands the shell never
sets `GH0ST_SHELL_FD`, and apps draw their own bars. The SDK already does that
for Rust apps.

An app can ask the shell to draw the OS top bar (title, network, battery and
clock) and an OS bottom bar (soft keys or key hints). The app then draws only
the rows between them. The shell watches the status sources for every app, so
an app needs no timers and reads no status files.

An app that never asks sees no change. Stock apps draw all 170 rows, as they
always did.

### Transport

- Every app gets one end of a `socketpair(AF_UNIX, SOCK_SEQPACKET)`.
  `GH0ST_SHELL_FD` holds its fd number. Without that variable there is no
  shell, and the app draws its own bars.
- Set `FD_CLOEXEC` on the fd once you have it, so your children don't inherit
  it.
- The socket has no path and no listener. Only the app holds its end.
- Nothing privileged goes over it. Root actions belong to the root helper.

### Messages

Each packet carries one message, in UTF-8, at most 512 bytes. Tabs separate
the fields: `verb<TAB>arg<TAB>arg`. Both sides ignore verbs they don't know.

App to shell:

| Message | Meaning |
|---|---|
| `bars` `top`, `bottom`, `both` or `none` | Which OS bars to draw |
| `title` text | The top bar's title. The default is the app's `Name` |
| `softkeys` slot4 ... slot8 | Five soft keys, each `icon:label`. Either part may be empty. An empty slot is `:` |
| `hints` `key=label` ... | Key hints instead of soft keys, such as `Enter=open` |
| `softkey` n `on` or `off` | Shows soft key n (4 to 8) lit, for a toggle |
| `toast` text | A short message over an OS bar for 2 s |

`icon` names a symbol built into gh0st, the Lucide set that `gh0st:<name>`
uses.

Shell to app:

| Message | Meaning |
|---|---|
| `hello` `1` | The protocol version, sent first |
| `content` x y w h | The rect the app may draw in, sent at start and after every `bars` change |
| `theme` name | The palette changed. Redraw with it |
| `redraw` | The shell drew over the content. Redraw all of it |
| `pause`, `resume` | Reserved for background apps |

### Rules

- Draw only inside the last `content` rect. After a `bars` change, keep
  drawing in the old rect until the new `content` arrives.
- The shell sends without blocking. If the app doesn't read, the shell keeps
  only the latest `content` and `theme` and sends them later.
- Send with `MSG_NOSIGNAL`. If the shell goes away, keep running and draw your
  own bars.
- The shell shortens long titles and toasts, and drops toasts that come too
  fast.
- `X-Gh0st-Bars=top`, `bottom`, `both` or `none` in the `.desktop` file draws
  the bars from the first frame, before the app has sent anything.

### Soft keys

The screen sits right above the number keys 4 to 8, so the five bottom slots
belong to those keys. Stock apps follow the same convention. Camera maps 4 to
back, 5 to zoom out, 6 to the shutter, 7 to zoom in and 8 to the gallery. The
keys send plain `KEY_4` to `KEY_8`, and the app reads them itself like every
other key.

- Slot 4 is back by convention. ESC does the same.
- In text entry the digits must type. Switch the bottom bar to `hints`, or
  turn it off, while a text field has focus.
- Slot centres follow the keys. Stock puts them at x = 49, 105, 161, 217 and
  273. Ours are in `theme.rs`.

## Leaving

- Exit normally. The shell returns when the whole process group is gone, so
  a background child keeps the app open.
- When the user holds ESC, the shell shows a hint at 0.5 s, sends SIGTERM to
  the group at 3 s, and sends SIGKILL 3 s later. Apps should exit cleanly on
  SIGTERM.
- The shell repaints the whole panel when the app is gone.

## Terminal

- The terminal command comes from `gh0st.terminal` in config.json. The default
  is `/usr/bin/gh0st-term -e`.
- The shell runs a `Terminal=true` entry as `<terminal> /bin/sh -c "<Exec>"`.
- With `GH0ST_SYSPLAUSE=1`, the terminal waits for a key after the command
  exits, then exits.

## Config

The shell and stock apps share `~/.config/cardputerzero/config.json`. The
shell re-reads it after every app exits, so a change an app made takes effect
on the home screen then.
