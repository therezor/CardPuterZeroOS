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
  it directly. An app must not grab it either, or Fn+K, ESC held, the volume
  keys and the screen timeout stop working. The shell takes the keyboard only
  once the app has left the front: paused, dark or locked (see Pausing).
- While an app is in front, the shell draws over it only for a banner across
  the top (ESC held, or the battery about to run out), a toast for the
  brightness, volume, mute and screenshot keys, and the OS bars an app asks
  for (see Shell link). When one goes, the shell puts back the app's pixels,
  except any the app drew meanwhile.
- The shell acts on the brightness, volume, mute and screenshot keys meanwhile.

## Shell link

The shell handles `hello`, `pause`, `paused`, `resume`, `redraw`, `toast`,
`awake` and `open`. It does not draw the OS bars, and that is not planned:
`bars`, `title`, `softkeys`, `hints`, `softkey`, `content`, `theme` and
`X-Gh0st-Bars` are not built. Apps draw their own bars, as the SDK does for
Rust apps.

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
| `awake` `1` or `0` | Keep the screen on while the app is in front, for a player or a clock. `0` lets it go dark again |
| `open` name | Bring up app `name` (its desktop file stem, or the program its `Exec` runs). Only the app in front may ask. It pauses, and comes back when `name` closes |
| `sound` name | Play a system sound: `move`, `select`, `back`, `open`, `close`, `lock`, `unlock`, `error`, `notice`, `warn` or `boot`. Only the app in front may ask, and only while the user has sounds on (`gh0st.sounds`): the app checks the setting, so a change in Settings counts at once. The shell plays nothing while dark or muted |

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

### Chips

The screen sits right above the number keys 4 to 8, so the bottom bar's
slots belong to those keys. `design.md` has the rules; in short:

- 4 is Esc and has no chip, as in the template apps. 5, 6 and 7 hold the
  page's actions. 8 is the app's menu.
- No chip repeats Enter or Esc.
- The keys send plain `KEY_4` to `KEY_8`, and the app reads them itself like
  every other key.
- In text entry the digits must type. Show key hints instead of chips while
  a text field has focus.
- Slot centres follow the keys. Stock puts them at x = 49, 105, 161, 217 and
  273. Ours are in `theme.rs`.

## Notifications

A notice is a small file, so any app, in any language, posts one without a
daemon, and it waits even when the app is gone. The shell watches two
folders with inotify:

- `~/.local/state/gh0st/notify/` for apps,
- `/var/lib/gh0st/notify/` for system services.

The file name is `<app>.<tag>`: `app` is the desktop file stem or the
program name, `tag` names the notice. Both are lowercase letters, digits,
`-` and `_`. Posting the same name again replaces the notice. Write a hidden
file (a name starting with `.`) and rename it, so the shell never reads half
of one. The body is `key=value` lines:

| Key | Holds |
|---|---|
| `title` | One line, up to 60 characters. Without it the file is not a notice |
| `body` | One line under it, up to 120 |
| `tone` | `info`, `ok`, `warning` or `critical` |
| `time` | Seconds since the epoch. The newest shows first |
| `open` | The app Enter opens, named as `app` is. Empty: the poster |
| `count` | A number for the app's tile, such as how many updates |

The shell toasts a new notice over whatever is in front, keeps it in the
notifications (5 in the switcher, or home's menu), lights a bell in its top
bar, and puts the count, or how many notices the app has, on its tile.
Clearing an app's notice deletes its file; clearing a system one hides it
until it is posted again with a new `time`. While locked, the lock screen
says only how many there are.

## Session services

The shell runs these on the session bus before any app starts
(`docs/keyring.md`):

| Name | What it does |
|---|---|
| `org.freedesktop.secrets` | The keyring. An app reads its own secrets; another app's take the user's yes on a shell screen |
| `org.freedesktop.Notifications` | Each notification becomes a notice, as above |
| `org.freedesktop.ScreenSaver` | Inhibit keeps the app's screen on, as `awake 1` does |

The keyring's questions show on a shell screen. The app in front is parked
for it, as Fn+K parks it, and gets `pause` and `resume` on its link.

## Pausing

- Fn+K (`KEY_HOME`) pauses the app in front and opens the switcher. On the
  home screen a tap of Tab opens it too, and closes it. Tab held 3 s on home
  or in the switcher locks; over an app, Tab is the app's. The app
  sees Fn and Home pressed and released; the shell waits until every key is
  up, then sends SIGSTOP to the group.
- The shell keeps the app's last frame. Before it sends SIGCONT, it puts that
  frame back on the panel, so an app that redraws only what changed still
  looks right.
- The screen going dark pauses the app in front the same way, without the
  switcher. So does a lock. A key after a dark shorter than Lock after
  resumes the app once every key is up. Past Lock after it wakes the lock
  screen, and the app resumes after Tab then Enter (and the password, when
  asked). The key that wakes the screen never reaches the app.
- Dark comes `dark_time` seconds after the last key the shell saw. Before
  it pauses the app, the shell takes the keyboard. If the app holds it (an
  `EVIOCGRAB` of its own), the shell can't see keys, so it leaves the app
  alone and the screen on. An app that sent `awake 1` stays in front too.
- While apps are paused and the shell is in front, while it is dark, and
  while the lock screen shows, the shell grabs the keyboard. A background app
  never reads the password. Before an app resumes, the shell lets go and writes `MSC_SCAN`
  events to the device until every paused reader's queue overflows, so the
  app reads `SYN_DROPPED` instead of keys typed elsewhere. Ignore `EV_MSC`.
- A stopped app's sound is parked. A stopped app leaves its streams open,
  which keeps the codec and PipeWire running. `gh0st audio park` moves its
  uncorked playback and capture streams to a null sink of its own, and the
  codec sleeps. On resume, or once the app is gone, the sink is unloaded
  and PipeWire moves the streams back to the default device. An app that
  runs behind is not parked.
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
- An app on the link gets `pause` when it leaves the front, whether it may run
  behind or not. The shell waits up to 100 ms for `paused`. The app then draws
  nothing and reads no keys. One that may run behind goes on, and the shell
  keeps no copy of its frame and doesn't guard the panel. Any other is then
  stopped, so it first lets go of what a stop would freeze: a camera
  streaming, a file half written. On return the shell sends `resume` and the
  app draws all of its screen.
- An app off the link, or one that doesn't answer, is guarded as before.
  While the shell is in front it gets no keys (the shell holds the grab),
  and the shell repaints over anything it draws, keeping its pixels for its
  return. Nothing tells the shell when another process writes the panel, so
  it looks every 500 ms while such an app runs and the screen is lit: the
  one check that polls. An app on the link never needs it. While another app is in front it reads that app's keys and its
  drawing shows over it.
- Apps run through `sudo` belong to root and cannot be paused; Fn+K says so
  and leaves them in front. One keeps the screen on too: the shell can't stop
  it drawing over the lock screen.

## The app menu

Every app has one menu, and every SDK app gets it built from its commands
(`sdk.md`, Commands). Key 8 opens it, from a chip that is always there.
Fn+H, which sends `KEY_HELP` (138), opens Help: every key of the page. The
OS acts on neither. Apps off the SDK should draw both and have them behave
the same.

**Opening and closing**
- 8 opens the menu over the app and closes it again; its chip is lit while
  it is open. While a field has focus 8 types, so Help, then 8, opens it.
- Esc steps back one level: from Help to the menu, from the menu to the
  page. Never open a menu with plain Esc: Esc is back.
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
- A row that loses work asks first: the first press shows "Press again".
  Close app always asks.
- A dimmed row can't be used now, as Paste with nothing copied.

**What goes in it**
- The app's actions and settings, most used first: about five show without
  scrolling. Group them under short headings.
- Settings the user changes often go in the menu as toggles and choices.
  The rest stay on the app's settings page, opened from the menu.
- The SDK adds Help and Close app last.

**Help**
- Help lists the chips on screen, what Esc and Enter do, every key of the
  page grouped by the mode it works in, and the keys that work in every app.
  F and X scroll it; Esc goes back to the menu, or to the page.
- It comes from the same list as the menu and the chips, so they never
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
| `Fn h` | Help |
| `r`, `t` | Smaller, larger (text size) |
| `v`, `p` | Copy, paste |
| `o`, `w` | New, close (tab, document) |
| `b` | Top bar |
| `u` | Refresh: scan, reload, check |

## Leaving

- Exit normally. The shell returns when the whole process group is gone, so
  a background child keeps the app open.
- On an app's first page, Esc held 0.7 s closes it: the app does this
  itself, after a "Hold to exit" toast. SDK apps get it for free.
- For an app that doesn't answer, the shell shows a banner at 1.5 s, sends
  SIGTERM to the group at 3 s, and SIGKILL 3 s later. Apps should exit
  cleanly on SIGTERM.
- The shell repaints the whole panel when the app is gone, and when it is
  paused.

## Terminal

- The terminal command comes from `gh0st.terminal` in config.json, a key set
  by hand: no screen sets it. The default is `/usr/bin/gh0st-term -e`, built
  from `crates/gh0st-term`.
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
