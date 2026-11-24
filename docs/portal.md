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

## Requests

Every method answers at once with a request object, then sends its
`Response(u, a{sv})` signal there when the user is done:

- The path is
  `/org/freedesktop/portal/desktop/request/SENDER/TOKEN`. SENDER is the
  caller's unique name without the `:` and with `.` as `_`. TOKEN is the
  `handle_token` option, or one of ours. Subscribe to it before calling.
- The response is `0` done, `1` cancelled, `2` anything else.
- `Request.Close()` gives up the request. No response follows.
- A caller that leaves the bus cancels its requests.

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
  (`apps.md`, Opening files), as Files' Enter does. `ask` shows the
  Open-with list first. `OpenDirectory` opens its folder in Files.
- `OpenURI` takes `file://` and acts as `OpenFile`. The spec sends those to
  `OpenFile`; we take them anyway. Other schemes answer 2: there is no
  browser. `SchemeSupported` is true for `file` alone.
- `gh0st open <path>` calls `OpenURI` from the terminal.

## Who may ask

The shell decides, as it does for the keyring's questions:

- Only the app in front, or a program it started, gets a chooser or opens
  an app. Anything running behind gets `2`.
- Not while the device is locked or dark: the request waits.
- The caller is parked as Fn+K parks it, and comes back when the chooser or
  the opened app closes.

## Limits

- No document portal: a picked file is its real path, as the upstream
  portal gives any app that is not sandboxed. Apps are not sandboxed yet
  (`roadmap.md`, App isolation).
- `parent` (a window handle) is ignored: the caller is always the app in
  front.
- `modal` is always true.

## Tests

`crates/gh0st-session/tests/compat.sh` drives each interface with `gdbus`
and a Python Gio client under `dbus-run-session`. It needs a Linux session
bus: CI, the image container or the device.
