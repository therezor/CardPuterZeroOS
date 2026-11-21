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
- A system app is one in `/usr/share/gh0st/applications` with a `gh0st:`
  icon, such as Settings and Terminal. Its symbol draws in the theme's
  accent on the same neutral badge as a PNG icon. Every other symbol or
  letter badge takes a tint picked from the app's file name.
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
  | `CAMERA_APP_KEYBOARD_GRAB` | `false`: the stock camera grabs the keyboard otherwise |

- The shell never grabs the keyboard while the app is in front. The app reads
  it directly. An app must not grab it either, or Fn+K, ESC held and the
  volume keys stop working.
- While an app is in front, the shell draws over it only for a banner across
  the top (ESC held, or the battery about to run out), a toast for the
  brightness, volume, mute and screenshot keys, and the OS bars an app asks
  for (see Shell link). When one goes, the shell puts back the app's pixels,
  except any the app drew meanwhile.
- The shell acts on the brightness, volume, mute and screenshot keys meanwhile.

## Shell link

Status: the transport and `hello`, `pause`, `paused`, `resume`, `redraw`
and `toast` are built. The bars are not yet (roadmap A1): apps still draw
their own, as the SDK does for Rust apps.

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
- It is `SOCK_SEQPACKET` on Linux. A host without it gets datagrams, which keep
  message bounds too.
- Nothing privileged goes over it. Root actions belong to the root helper.

### Messages

Each packet carries one message, in UTF-8, at most 512 bytes. Tabs separate
the fields: `verb<TAB>arg<TAB>arg`. Both sides ignore verbs they don't know.

App to shell:

| Message | Meaning |
|---|---|
| `hello` `1` | The app speaks the link. Without it the shell treats the app as stock |
| `paused` | The answer to `pause`: the app draws nothing and reads no keys now |
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
| `pause` | The app runs behind now. Stop drawing, close the keyboard, answer `paused` |
| `resume` | The app is in front again. Open the keyboard and draw all of the screen |

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

## Pausing

- Fn+K (`KEY_HOME`) pauses the app in front and opens the switcher. On the
  home screen a tap of Tab opens it too, and closes it; Tab held locks. The app
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
- The switcher shows each app's memory: PSS plus swapped PSS, summed over
  its group and every descendant, so a terminal's inner shell counts. A
  paused app is read once per pause, and the figure shows at once on the
  next open. An app running in the background is read every second while
  the switcher shows, with its CPU share. The first CPU figure comes 250 ms
  after opening. Nothing is read while the switcher is closed.
- An app allowed to run in the background (Settings > Apps) is not stopped.
  `X-Gh0st-Background=true` in its `.desktop` file makes that the default for
  an app that answered `hello`.
- An app on the link gets `pause` when it leaves the front. The shell waits up
  to 100 ms for `paused`. The app then draws nothing and reads no keys, so
  the shell keeps no copy of its frame and doesn't guard the panel. On return
  the shell sends `resume` and the app draws all of its screen.
- An app off the link, or one that doesn't answer, is guarded as before.
  While the shell is in front it gets no keys (the shell holds the grab),
  and the shell repaints over anything it draws, keeping its pixels for its
  return. While another app is in front it reads that app's keys and its
  drawing shows over it.
- Apps run through `sudo` belong to root and cannot be paused; Fn+K says so
  and leaves them in front.

## The app menu

Every app has one menu, and every SDK app gets it built from its commands
(`sdk.md`, Commands). Fn+Esc opens it, and so does Fn+H, which sends
`KEY_HELP` (138). The OS acts on neither and no terminal program reads them,
so they mean the same thing in every app. Apps off the SDK should draw one
that behaves the same.

**Opening and closing**
- Fn+Esc or Fn+H opens the menu over the app and closes it again.
- Esc closes it without acting. Never open a menu with plain Esc: Esc is
  back, and terminal programs need it.
- It opens at the top, on the most used row. Right after a setting was
  changed in place, it opens on that setting, to change it again.
- F and X (or Fn with them) move, wrapping at the ends.

**Rows**
- An action closes the menu, then acts. A row that opens a page shows a
  chevron.
- A toggle flips in place on Enter, Z or C. A choice shows `‹ value ›` and
  steps in place on Z and C; Enter steps forward. The menu stays open and
  shows the new value, so a setting can be tried and tried again.
- A row's key shows in a pill on the right. Pressing it while the menu is
  open picks the row.
- A row that loses work asks first: the first Enter shows "Enter again".
  Close app always asks.
- A dimmed row can't be used now, as Paste with nothing copied.

**What goes in it**
- The app's actions and settings, most used first: about five show without
  scrolling. Group them under short headings.
- Settings the user changes often go in the menu as toggles and choices.
  The rest stay on the app's settings page, opened from the menu.
- The SDK adds Keys and Close app last.

**The Keys screen**
- Keys opens a screen with every key of the page in front, grouped by the
  mode it works in, and the keys that work in every app. F and X scroll it,
  and Esc goes back to the menu.
- It comes from the same list as the menu and the hint bar, so they never
  disagree. An app with modes names each one: Typing, Scroll mode, Find.

**Keys**
- Letters and digits. Symbols need an extra press of Sym.
- Lowercase in pills; an uppercase letter means Shift with it.
- Not F, X, Z or C: those are the arrows on a page that isn't typing.
- The same letter for the same thing in every app:

| Key | Item |
|---|---|
| `s` | Settings |
| `n` | Find |
| `h` | Keys |
| `r`, `t` | Smaller, larger (text size) |
| `v`, `p` | Copy, paste |
| `o`, `w` | New, close (tab, document) |
| `b` | Top bar |

## Leaving

- Exit normally. The shell returns when the whole process group is gone, so
  a background child keeps the app open.
- When the user holds ESC, the shell shows a hint at 0.5 s, sends SIGTERM to
  the group at 3 s, and sends SIGKILL 3 s later. Apps should exit cleanly on
  SIGTERM.
- The shell repaints the whole panel when the app is gone, and when it is
  paused.

## Terminal

- The terminal command comes from `gh0st.terminal` in config.json. The default
  is `/usr/bin/gh0st-term -e`, built from `crates/gh0st-term`.
- The shell runs a `Terminal=true` entry as `<terminal> /bin/sh -c "<Exec>"`.
- With `GH0ST_SYSPLAUSE=1`, the terminal waits for a key press after the
  command exits, then exits.
- Programs in it see `TERM=xterm-256color` and `COLORTERM=truecolor`. Each
  runs in its own session, so pausing the terminal's group never stops them,
  and closing the terminal hangs them up. A program that ignores SIGHUP
  outlives the terminal and is not counted in the switcher's memory figure.
- The terminal runs in the background by default and reads its programs'
  output there, so a build doesn't stall while another app is in front.

## Config

The shell and stock apps share `~/.config/cardputerzero/config.json`. The
shell re-reads it after every app exits, so a change an app made takes effect
on the home screen then.
