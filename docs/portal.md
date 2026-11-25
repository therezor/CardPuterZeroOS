# The portal

`gh0st session` serves the XDG desktop portal on the session bus, as
`org.freedesktop.portal.Desktop` at `/org/freedesktop/portal/desktop`. It is
our own, not the upstream `xdg-desktop-portal`: no extra daemons, nothing
more at idle. Any portal client works: our SDK apps, SDL3 (its dialogs ask
the portal by default), libportal, ashpd. GTK and Qt need a display server,
which the device has none of.

## Interfaces

| Interface | Version | What it does |
|---|---|---|
| `FileChooser` | 4 | `OpenFile`, `SaveFile`, `SaveFiles`, through Files as the chooser |
| `OpenURI` | 5 | `OpenFile` and `OpenDirectory` by fd, `OpenURI` for `file://` |
| `Notification` | 2 | `AddNotification` and `RemoveNotification`: a notice, its buttons as chips |
| `Settings` | 2 | The look: light or dark, the accent, reduced motion |
| `Inhibit` | 3 | Keeps the screen on; `CreateMonitor` reports the session |
| `Account` | 1 | The user's name, after the user's yes |
| `Trash` | 1 | `TrashFile` moves a file to the Trash |
| `Wallpaper` | 1 | Sets the wallpaper, after the user's yes over the picture itself |
| `MemoryMonitor` | 1 | `LowMemoryWarning` when memory runs short |
| `Screenshot` | 2 | `Screenshot` after the user's yes over the frozen screen; `PickColor` with a crosshair |
| `NetworkMonitor` | 3 | The network as NetworkManager sees it, and `changed` |
| `PowerProfileMonitor` | 1 | `power-saver-enabled`: Settings' CPU mode is Power Saver |

## Requests

A method that returns a request object answers at once, then sends its
`Response(u, a{sv})` signal there when the user is done:

- The path is
  `/org/freedesktop/portal/desktop/request/SENDER/TOKEN`. SENDER is the
  caller's unique name without the `:` and with `.` as `_`. TOKEN is the
  `handle_token` option, or one of ours. Subscribe to it before calling.
- The response is `0` done, `1` cancelled, `2` anything else.
- `Request.Close()` gives up the request. No response follows.
- A caller that leaves the bus cancels its requests.
- A request the shell can't be told about, such as a path too long for
  the shell link, answers `2`.
- Request ids start at a random number, so an answer meant for a session
  that died reaches no request of the next.

## FileChooser

Options:

| Option | Type | Use |
|---|---|---|
| `handle_token` | s | The request's TOKEN |
| `accept_label` | s | The main chip's word |
| `multiple` | b | Pick several |
| `directory` | b | Pick a folder |
| `filters` | a(sa(us)) | Named filters: kind 0 a glob, 1 a MIME type |
| `current_filter` | (sa(us)) | The filter shown first |
| `current_name` | s | The name a save starts with |
| `current_folder` | ay | Where to start, NUL-ended |
| `current_file` | ay | A file to save over |
| `choices` | a(ssa(ss)s) | Taken and answered unchanged |

The response carries `uris` (`as`, `file://`), `current_filter` and
`choices`.

The chooser is Files (`files.md`, The file chooser), started by the shell
over the asking app with `gh0st-files --pick <id>`. It gets the question
from `gh0st session` with `org.gh0st.Portal.Take` and answers with
`Done`; the session checks it is the process the shell started. Paths never
travel the shell link.

## OpenURI

- `OpenFile(s parent, h fd, a{sv})` opens the file the fd names in its app
  (`apps.md`, Opening files), as Files' Enter does. `OpenDirectory` opens
  its folder in Files. `ask` is not offered yet: the default app opens.
- `OpenURI` takes `file://` and acts as `OpenFile`. The spec sends those to
  `OpenFile`; we take them anyway. Other schemes are refused with
  `InvalidArgs`: there is no browser. `SchemeSupported` is true for `file`
  alone.
- `gh0st open <path>` calls `OpenURI` from the terminal.

## Notification

`AddNotification(s id, a{sv})` posts a notice (`apps.md`, Notifications):
`title`, `body` or `markup-body` (tags dropped), `priority` (`urgent` is
critical, `high` a warning), `default-action` with its target for Enter,
and up to three `buttons` (`label`, `action`, `target`) as chips. The same
id again replaces it; `RemoveNotification(id)` takes it away. A picked
button or Enter sends `ActionInvoked(id, action, [target])` to the app
that posted it.

## Settings

`ReadAll`, `ReadOne` and the older `Read` serve
`org.freedesktop.appearance` from the shared config:

| Key | Value |
|---|---|
| `color-scheme` | `1` dark, `2` light (Paper) |
| `accent-color` | The palette's accent, `(ddd)` from 0 to 1 |
| `contrast` | `0` |
| `reduced-motion` | `1` when animations are off in Settings |

A change to config.json sends `SettingChanged` for each key that moved.

## Inhibit

- `Inhibit` with flag `4` (suspend) or `8` (idle) keeps the screen on, as
  `awake 1` does for the app the caller belongs to. Flags `1` and `2` hold
  nothing: there is no logout and no user switch.
- It answers `0` at once and holds until `Request.Close`, or until the
  caller leaves the bus.
- An app's holds count together, with ScreenSaver's too, across all its
  processes: the screen may dim once the last one ends.
- `CreateMonitor` answers with a session at
  `/org/freedesktop/portal/desktop/session/SENDER/TOKEN` and one
  `StateChanged`: running, the screen saver off. `Session.Close` ends it.
  `QueryEndResponse` does nothing.

## Account

`GetUserInformation` answers `id` (the login name chosen at setup), `name`
(the full name in passwd, else the login) and an empty `image`. The shell
asks first: "<App> wants to see your name", Once on 5 or Always on 7. Esc
denies (`1`). Always is kept in
`~/.local/state/gh0st/portal-always`.

## Trash

`TrashFile(h fd)` moves the file the fd names to
`~/.local/share/Trash`, with the `.trashinfo` Files and other file managers
read to put it back. The name must still be the file the fd holds. It
answers `1` done, `0` not: a folder on another drive has no Trash here.

## Wallpaper

- `SetWallpaperFile(s parent, h fd, a{sv})` and `SetWallpaperURI` with a
  `file://` URI. Other schemes are refused: there is no download.
- Only the app in front, or one with Always, may ask. The URI's file is
  opened after that check, and only a regular file: a FIFO or a device is
  refused unopened.
- `gh0st-settings --import-wallpaper - TMP` fits the picture to the screen
  in a child, reading the fd as its stdin. A PNG is read directly; any
  other picture goes through ffmpeg first.
- The shell then asks "Use as wallpaper?" over the fitted picture, Once on
  5 or Always on 7, Esc to deny. A yes keeps it as Settings' own picture.
- `show-preview` and `set-on` are taken and ignored: the question always
  shows the picture, and home and the lock share one wallpaper.
- Always is kept with Account's, in `~/.local/state/gh0st/portal-always`,
  one `what<TAB>id` a line: the app's desktop id, or `exe:` and the path
  of a program outside apps. Such a program skips the question only while
  it is in front, since every script of an interpreter shares its path.

## Screenshot

- `Screenshot(s parent, a{sv})`: the shell freezes the screen as the app
  left it, without its own overlays, and asks "Share this screenshot?" over
  it, Once on 5 or Always on 7. A yes saves it to `~/Pictures/Screenshots`
  and answers `uri`. Always is kept with Account's; the next one is saved
  without asking. `interactive` and `modal` are taken and ignored.
- `PickColor(s parent, a{sv})`: the frozen screen, whole, with a crosshair
  in the middle. The arrows move it, Shift steps 10. A card shows the
  colour under it and its hex, away from the crosshair. Enter answers
  `color` as `(ddd)` from 0 to 1; Esc answers `1`. Picking is the user's
  yes: there is no question first.
- Each waiting question keeps its own frozen screen, as a wallpaper
  question keeps its picture. Four may wait at once; past that the shell
  denies.

## NetworkMonitor

- `GetStatus`, `GetAvailable`, `GetMetered` and `GetConnectivity` read
  NetworkManager on the system bus. The connection opens on the first
  call, so a session no app asks keeps none. Its PropertiesChanged signals
  then wake the loop, and `changed` goes out when the answer moves. Only
  signals from NetworkManager's own name count.
- Without NetworkManager the connection stays open. When it starts, its
  NameOwnerChanged wakes the loop and it is read then.
- Connectivity is NetworkManager's, as `1` local, `2` limited, `3` captive
  portal, `4` full. Its check is off on the image, so its state stands in:
  connected to the internet is `4`, to a site `2`.
- Metered is NetworkManager's yes or guess-yes.
- `CanReach` answers whether the network is there: no name is looked up.
- Without NetworkManager, the network is there and full.

## PowerProfileMonitor

`power-saver-enabled` is true while Settings' CPU mode is Power Saver (the
`powersave` governor). Settings keeps the mode as a udev rule in
`/etc/udev/rules.d`; a change there sends PropertiesChanged.

## MemoryMonitor

PSI triggers on `/proc/pressure/memory` wake the loop when tasks stall on
memory. In a 2 s window: 140 ms of some tasks stalled sends level `50`, 300
ms sends `100`, and 300 ms of all tasks stalled sends `255`. Each sends
`LowMemoryWarning(y level)` to everyone on the bus, and the shell sends
`lowmem <level>` to every linked app (`sdk.md`, Low memory). The image
turns PSI on with `psi=1`; without it the monitor is quiet.

## Who may ask

The shell decides, as it does for the keyring's questions:

- Only the app in front, or a program it started, gets a chooser, opens
  an app, or asks the user about their name, the wallpaper, a screenshot
  or a colour. Anything
  running behind gets `2`, or `1` for those questions.
- A program is the app's when the app is among its parents. Joining the
  app's process group does not count, and neither does a program left to
  the shell when its parent exited.
- Not while the device is locked or dark: the request waits.
- The caller is parked as Fn+K parks it, and comes back when the chooser or
  the opened app closes.

## Limits

- No document portal: a picked file is its real path, as the upstream
  portal gives any app that is not sandboxed. No app here is sandboxed.
- `parent` (a window handle) is ignored: the caller is always the app in
  front.
- `modal` is always true.

## Tests

`crates/gh0st-session/tests/compat.sh` drives each interface with `gdbus`
and a Python Gio client under `dbus-run-session`. It needs a Linux session
bus: CI, the image container or the device.
