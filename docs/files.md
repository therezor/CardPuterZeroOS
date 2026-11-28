# Files

Files browses the card, opens archives as folders, and reaches SFTP, FTP,
FTPS and WebDAV servers. It copies, moves, renames and deletes between any
two of them. It replaces M5's stock Files app.

`cargo run -p gh0st-files --example mockups` draws every screen into
`mockups/files`.

## Copy, Cut, Paste

The screen holds one full-width list. Copy (5) or Cut (7) takes what is
marked, or the row under the cursor, and nothing moves yet. Open any folder,
on the card or a server, and Paste (6) brings it there: a copy, or a move
for Cut. The top bar says what waits ("2 copied", "1 cut"); a cut item
shows grey in its folder until you paste it. A copy pastes again and again; a
cut pastes once. Pasted into its own folder, a copy takes a new name
("photo 2.png"). Paste is grey while nothing waits. Ctrl+C, Ctrl+X and
Ctrl+V do the same.

From inside an archive, Copy and then Paste extract what was taken.

There are two sides, A and B, each with its own place, folder, selection and
marks: two places open at once. Tab flips between them, and the top bar
names the side and its folder: `A · ~/Music`. The clipboard serves both.

Each side's root is **Places**: Home, Card (`/`), Downloads, Recent, Trash,
every saved server, and Add server. Side A opens at Home, side B at Places.

Every folder starts with a ".." row: Enter on it goes up, as Esc does. A
folder opens with the cursor on its first entry, one Up below "..". A row
shows a type icon, the name, and the size on the right; a folder shows a
chevron instead. Folders come first. The type, from the name, picks
the icon and what Enter does:

| Type | Icon | Enter |
|---|---|---|
| Folder, link to a folder | folder | opens it |
| zip, tar and its compressions, 7z, rar, iso | file-archive | opens it as a folder |
| Any other file on the card | file-text, file-code, file-image, file-music, file-video, file | opens it in its app (`apps.md`, Opening files) |
| Text no app opens | file-text, file-code | the text viewer, first 256 KB |
| Anything else | file | Info |

A folder lists at most 20,000 entries, with a last row that says the rest is
not shown.

## Keys

| Key | Does |
|---|---|
| Enter, Right | Opens the row: a folder, an archive, or a file in its app |
| `o` | Open with: the apps that open the file, the default first. 6 Always makes the pick the default |
| Esc | Up a folder. Clears marks or a search first. At Places, held 0.7 s, exits |
| Left | Up a folder |
| Tab | The other side |
| Space, 4 | Marks the row and moves down |
| Ctrl+C, Ctrl+X, Ctrl+V | Copy, Cut, Paste |
| `a` | Marks all, or none |
| `n` | Finds in the folder as you type |
| `r` | Rename |
| `k` | New folder |
| Del (Fn+Bksp) | Delete |
| `i` | Info: kind, size, changed, access, link target, folder |
| `e` | Extract an archive here, into a new folder named after it |
| `t` | Transfers |
| Fn+L, Fn+M, Fn+N | A screen up, a screen down, the last row |

Chips: 4 Mark, 5 Copy, 6 Paste, 7 Cut, 8 the menu. Mark takes key 4 from
Esc, which still goes up a folder. They stay on their keys; Paste is grey
until something waits. An empty folder shows Paste alone, when something
waits. Inside an
archive the keys stay put, with Paste and Cut grey. On a server's row in Places: 5 Edit, 7
Forget, pressed twice.

The menu (≡) also has Mark, Clear the clipboard, Compress to zip, Same folder on both sides, Sort by
(name, size, date, type), Hidden files, Add server, SSH key and Empty the
Trash.
Sort and hidden files are kept in `files.sort` and `files.hidden`.

## Asking first

- **Delete on the card** moves to `~/.local/share/Trash`, the freedesktop
  Trash, after a plain dialog. The Trash undoes it.
- **Delete on a server, in the Trash, and Empty the Trash** can't be undone:
  the dialog's yes is a red chip on 7, held 0.7 s. Enter does nothing.
- **A name already in the target** stops the copy on a page: Replace it,
  Keep both, Skip it, and Same for the rest. Esc skips.
- **Plain FTP and WebDAV over http** send the password and the files where
  anyone on the network can read them. Places tags them in `warning`, and
  the top bar says "Not private" while a side is on one.
- **Open as root** (the menu, or Ctrl+Enter) asks for the user's password,
  then lists and changes files on the card as root. The top bar says
  "Root" in `critical` until Leave root, until Files goes behind or the
  screen locks (after a copy under way ends), or until it closes. Deletes
  are for good, behind the held red chip, and Open with is off: the app
  it would open runs as the user.

## Root mode

The page starts a second worker through `gh0st-helper files-worker`, the
helper verb that hands over a root worker after the password (5 minutes of
grace, as for every verb that asks). The worker greets the page before it
takes a request, which is how the page knows the password was right. Every
local listing, copy, move, delete, rename, new folder and the viewer go to
it; servers and archives stay with the user's worker.

- It does files on the card only. It refuses servers, archives, the SSH key
  and the Trash, so ssh, curl, bsdtar and ssh-keygen never run as root.
- In the user's own folders it acts as the user. Each call looks at the
  path's folders from `/` down: a folder the user owns, or one its group or
  anyone may write to, makes all below it the user's, and there the worker
  takes the user's ids for that call. Files it makes there are the user's.
  A folder only root can change can't be swapped mid-walk, so there it acts
  as root.
- Links are made again, never followed: a copy's source opens with
  `O_NOFOLLOW`, and a link in a folder is made anew as whoever its folder
  belongs to.
- It is as strong as sudo, behind the same password. Any program of the
  user's that runs outside a sandbox could drive Files while root mode is
  on.

## Servers

The server form is a list: Type, Address (`host`, `host:port` or
`[v6]:port`), User, Password, Remember it, Folder, Name. Connect on 6 saves
it and opens it on the current side. An SFTP server opens at the user's home
(`~`) unless Folder says otherwise; the others open at `/`.

| Type | Port | How |
|---|---|---|
| SFTP | 22 | Our SFTP client over `ssh -s sftp` |
| FTPS | 21 | curl, explicit TLS |
| FTP | 21 | curl, plain |
| WebDAV | 443 | curl, https |
| WebDAV http | 80 | curl, plain |

The list is `~/.local/share/gh0st-files/servers`, one tab-separated line
each: name, type, host, port, user, folder, remember. It holds no secret.

### Passwords and keys

Passwords live in the keyring (`docs/keyring.md`), never in a file. Each is
an item of schema `org.gh0st.Files.Server` with the attributes `protocol`,
`host`, `port` and `user`, labelled `<name> (<type>)`, so the Keyring app
lists it under Files.

- **Remember it** is on by default. When the server asks for its password,
  Files looks in the keyring first. Locked, the shell asks to unlock it, and
  Files waits on the bus.
- A password typed in the form or a dialog goes to the keyring only once a
  listing with it works. A wrong one is never kept, and a wrong kept one is
  asked for again.
- With Remember off, every connect asks.
- Forget removes the server and asks the keyring to delete its item.
- Only the page talks to the keyring, over one connection. The worker
  asks the page for a secret, and gets it once.

SSH has its own files in `~/.local/share/gh0st-files/ssh`, outside `~/.ssh`:
`known_hosts`, and a key. The menu's SSH key makes one, ed25519, sealed with
a random passphrase kept in the keyring, so the file alone opens nothing. It
then shows the public half as text and a QR code, for the server's
`authorized_keys`. When ssh needs the passphrase, Files answers from the
keyring without asking.

A server never seen before shows its key's SHA-256 fingerprint whole, in
groups of four, before anything is sent. A key that changed is refused, and
the dialog says someone may be in the way.

## Archives

bsdtar (`libarchive-tools`) reads zip, tar with gzip, xz, bzip2, lz4 or
zstd, 7z, rar and iso. Opening one lists its table of contents once; its
folders then come from that list, with no more reading. Copy inside one,
then Paste in a folder, extracts what was taken there. Extract here makes a
new folder next to the archive.

bsdtar refuses entries with `..` or an absolute path unless given `-P`, and
Files never passes it. Free space is checked before an extract. Compress to
zip writes a new zip of the marked items.

## How it is built

```text
page (gh0st-files) ──socket pair──> worker (gh0st-files --worker)
  local listings, keyring             servers, archives, copies
                                       ├─ ssh -s sftp     (pipes)
                                       ├─ curl            (one per call)
                                       └─ bsdtar          (one per call)
```

- **The page** lists a local folder itself: `readdir` is quick. Everything
  that can be slow goes to the worker, so the page never blocks.
- **The worker** is a child on a stream socket pair, joined to the page's
  loop. It starts on the first request and ends with the app. It runs one
  request at a time in one `poll` loop. While a request waits on the
  network or a child, the loop keeps reading the page, so answers and Stop
  arrive. Requests and events (`src/engine.rs`, `src/proto.rs`) carry an id,
  one line each; fields travel escaped, so tabs and newlines in names
  survive. Idle, it sleeps in `poll`.
- **SFTP** is our version 3 client (`src/worker/sftp.rs`) over `ssh -s
  sftp`, with `-F /dev/null` and Files' own known hosts and key. ssh's
  questions go to an askpass program, which is this binary again: it
  carries the prompt to the worker over a socket in a folder only the user
  can open (ssh closes inherited fds, so a path is the way through). The
  worker asks the page, and the page asks the keyring or the user. That
  askpass is Files' own; `sudo`'s goes through the shell (`isolation.md`).
- **FTP, FTPS and WebDAV** run curl once a call (`src/worker/curl.rs`). FTP
  lists with MLSD, and with LIST where a server lacks it. WebDAV lists with
  PROPFIND, read by a small scanner that ignores namespace prefixes.
- **Copies** go between any two places with progress: 64 KB chunks on the
  card, 32 KB requests over SFTP, curl's own transfer elsewhere. Between two
  servers a file waits on the card. A move on one filesystem is a rename;
  otherwise a copy, then the delete. One transfer runs at a time. The top
  bar shows its percent, and `t` shows the file, the bytes and a Stop chip.
- The app may run behind (`X-Gh0st-Background`), so a transfer goes on when
  it leaves the front. It keeps the device awake while one runs. Behind, a
  question it waits on, a failure and its end each post a notice.
- Idle, it sets no timer and wakes for nothing.
- A worker question never opens over a dialog of the user's own: it waits
  for that answer, so an Enter meant for one can't answer the other. A
  Delete, Rename or Compress acts on the names its dialog showed.

### What the worker refuses

- **A move deletes only what arrived.** A file skipped at a clash keeps its
  source, and so do the folders around it. Devices and pipes stay.
- **Links inside a folder are not followed.** On the card they are made
  again as links; going to or from a server they are left out. Only an item
  the user picked may be a link copied through. A folder that links to
  itself can't loop, and an archive can't reach files beside it.
- **Replace never deletes what holds the source.** Folder onto folder
  merges, on a copy and on a move.
- **Names that climb out are dropped.** A server's listing with an empty
  name, ".", "..", a slash or a NUL is a server lying.
- **Archive names are names.** bsdtar gets them after `--`, with `*`, `?`,
  `[` and `]` escaped, and is never given `-P`.
- **A folder is copied whole.** The 20,000 cap is the screen's; the worker
  lists all of it.

## Tests

- **Unit tests** cover types, natural order, sorting, folder steps, the
  archive tree, the server list round trip, addresses, and names.
- **`tests/flow.rs`** drives the page over a fixture home and the fake
  worker (`src/fake.rs`): copy and its target, the Trash, a held delete,
  Esc to Places, idle wakes, and passwords against the real Secret Service
  with no bus (kept once working, used without asking, never in the file).
- **`tests/worker.rs`** drives the real worker binary: copies, moves and
  conflicts, the Trash, a queued Stop, archives (one with an entry that
  climbs out), SFTP against the system's `sftp-server` with no network, and
  making the SSH key. With `GH0ST_FILES_SSH=user:password@host` it also runs
  real ssh: the host-key question, a wrong password asked again, and the
  key known the second time, as a trixie container with sshd runs it.
- **The mockups** are fixtures: every scene in every palette under the
  design lint.

## The file chooser

When an app asks for a file through the portal (`portal.md`), the shell
runs Files as its chooser over that app: a page of its own, not the Files
you may have open. The top bar says what it does and for whom: `Open ·
~/Documents` and `Text`.

- One pane: no second side, no Tab, no clipboard.
- It shows the card only: Home, Card, Downloads and, to open, Recent. No
  servers, no Trash, and archives are files.
- **Recent** lists the files opened lately by any app, newest first, of
  the types the app takes. Enter picks one. Files lists it among Places
  too, where Enter opens the file in its app.
- The app's file types hide the rest; folders always show. With more than
  one type, a Type chip picks which.
- **Open:** Enter picks the file. When the app takes several, 4 or Space
  marks them and Enter picks the marks. Only files are marked, Mark all
  too.
- **Save:** Save opens the name, filled with the app's. Enter on a file
  saves over it. Saving over a file asks first; a folder's name is
  refused.
- **Folder:** only folders show. Choose picks the one you are in.
- Esc held at Places, or Esc held 3 s anywhere, gives up: the app hears
  that no file was picked.

## Opening a file elsewhere

Enter on a file on a server or in an archive, when an app opens its type,
copies it to `~/.cache/gh0st-files/open/<hash>/` with the usual progress
card (Esc stops it), then opens it in its app. Enter again while it comes
does nothing more. The copy and its folder are read only: the app can't
save over it as if it were the original, nor rename a new file over it,
and offers Save as. A copy already there, the same size and time, opens at
once; a source with no time (some WebDAV servers) is copied again. Files
keeps 256 MB of copies, removing those opened longest ago when it starts,
never the newest, and not when it starts as a chooser.

## Limits

- No SMB: curl can't list an SMB folder.
- Root mode can't reach servers or archives, and a running delete of a big
  folder finishes even if the page goes: the shell can't signal a root
  worker, which ends when it reads its link closing.
- In a folder another system user owns, root mode acts as the user and may
  be refused, as that folder is not root's alone.
- A change to a file opened from a server or an archive stays on the
  card: save it as a new file and copy it back.
- An archive with a password doesn't open.
- A file no app opens shows its info; text without an app opens in the
  viewer.
