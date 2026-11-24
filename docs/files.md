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
shows grey in its folder until it moves. A copy pastes again and again; a
cut pastes once. Pasted into its own folder, a copy takes a new name
("photo 2.png"). Paste is grey while nothing waits. Ctrl+C, Ctrl+X and
Ctrl+V do the same.

From inside an archive, Copy and then Paste extract what was taken.

There are two sides, A and B, each with its own place, folder, selection and
marks: two places open at once. Tab flips between them, and the top bar
names the side and its folder: `A · ~/Music`. The clipboard serves both.

Each side's root is **Places**: Home, Card (`/`), Downloads, Trash, every
saved server, and Add server. Side A opens at Home, side B at Places.

A row shows a type icon, the name, and the size on the right; a folder
shows a chevron instead. Folders come first. The type, from the name, picks
the icon and what Enter does:

| Type | Icon | Enter |
|---|---|---|
| Folder, link to a folder | folder | opens it |
| zip, tar and its compressions, 7z, rar, iso | file-archive | opens it as a folder |
| Text and code | file-text, file-code | the text viewer, first 256 KB |
| Image, audio, video, other | file-image, file-music, file-video, file | Info |

A folder lists at most 20,000 entries, with a last row that says the rest is
not shown.

## Keys

| Key | Does |
|---|---|
| Enter, Right | Opens the row |
| Esc, Left | Up a folder. Clears marks or a search first. At Places, held 0.7 s, exits |
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
until something waits. An empty folder shows Paste alone. Inside an
archive: 4 Mark and 6 Copy. On a server's row in Places: 5 Edit, 7
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
  worker asks the page, and the page asks the keyring or the user.
- **FTP, FTPS and WebDAV** run curl once a call (`src/worker/curl.rs`). FTP
  lists with MLSD, and with LIST where a server lacks it. WebDAV lists with
  PROPFIND, read by a small scanner that ignores namespace prefixes.
- **Copies** go between any two places with progress: 64 KB chunks on the
  card, 32 KB requests over SFTP, curl's own transfer elsewhere. Between two
  servers a file waits on the card. A move on one filesystem is a rename;
  otherwise a copy, then the delete. One transfer runs at a time. The top
  bar shows its percent, and `t` shows the file, the bytes and a Stop chip.
- The app may run behind (`X-Gh0st-Background`), so a transfer goes on when
  it leaves the front. It keeps the device awake while one runs and posts a
  notice when it ends unseen.
- Idle, it sets no timer and wakes for nothing.

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

## Limits

- No SMB: curl can't list an SMB folder.
- A text file on a server opens once copied to the card.
- An archive with a password doesn't open.
- No editor, and no "open with": an app can't be started with a file yet.
- The viewer reads text. Images, audio and video show their info.
