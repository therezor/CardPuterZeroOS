# The keyring

`gh0st keyring` is the system keyring. It serves the freedesktop Secret
Service (`org.freedesktop.secrets`) on the session bus, so libsecret,
`secret-tool`, python `keyring`, git's libsecret helper, Chromium and every
other Linux app that keeps secrets use it unchanged.

`gh0st session` runs beside it with two more session services:

- `org.freedesktop.Notifications`: a notification becomes a notice file the
  shell toasts (`docs/apps.md`, Notifications). An app keeps eight at most.
- `org.freedesktop.ScreenSaver` Inhibit and UnInhibit: the app's `awake`, as
  if it had said it on its link. It ends with the last inhibitor or the
  connection.

The shell starts both on socket pairs before any app, and starts one again
the moment it dies, so no app gets time to take its bus names. A dead shell
takes them with it.

## The vault

One file, `/var/lib/gh0st/keyring/1000/login.vault`, outside the home
folder (`packaging/systemd/keyring`).

```text
magic "GH0STKR1" | version 1 | argon2id m t p | salt[16]
| wrap nonce[24] | wrapped data key[48] | body nonce[24] | sealed body
```

- A random 32-byte data key seals the body with XChaCha20-Poly1305. The body
  holds every label, attribute, secret and approval. Nothing but the header
  is in clear.
- Argon2id of the login password wraps the data key, with XChaCha20-Poly1305
  too. The wrap's AD is the header up to the salt; the body's is the whole
  header.
- Every write replaces the file whole: a temp file, fsync, rename, fsync of
  the folder.
- Argon2id costs are in the header. A file asking for more than 256 MiB or
  16 passes is refused before any work.
- The crypto is RustCrypto's `argon2` and `chacha20poly1305`, with `zeroize`.

## Unlock and lock

- The lock screen runs `unix_chkpwd`. When it says the password is right,
  the shell sends it to the keyring over its socket pair, never a bus.
- The vault opens on its own tag. A password the system didn't check never
  reaches it, and only a checked one can make a new vault.
- A device lock closes the vault: the data key and every secret go, except
  NetworkManager's Wi-Fi passwords.
- Attributes and labels stay in memory after the first unlock. A search
  while locked still finds the right items, reported locked, and the client
  asks to unlock them.
- With the lock password off, the shell asks for the password once after
  the unlock.

## Who is asking

- The shell runs each app in a cgroup `app-<desktop stem>` right under
  `gh0st.service`. That name is the app. Any other process is its
  executable's path.
- The pid comes from the bus, with a pidfd where dbus-daemon gives one, so a
  reused pid can't pass for another program.
- An item belongs to the app that stored it. To any other app it looks
  locked, `GetSecret` says `IsLocked`, and `Unlock` asks the user through
  the shell:
  - Enter allows once, for this connection.
  - 6 allows always; the vault keeps it.
  - Esc says no.
- The Keyring app shows and revokes approvals.

## Prompts

- A Secret Service prompt (`Unlock`, `Delete`, `CreateCollection`) asks on
  a shell screen while the shell holds `EVIOCGRAB`. The app in front is
  parked first, as Fn+K parks it.
- An app that can't be parked (run through sudo) or that holds the keyboard
  could read the answer, so the question is refused instead.
- A prompt's password goes through `unix_chkpwd` and the lock's backoff
  before the vault sees it.
- While the device is locked, prompts wait for the unlock.

## Password changes

- **Settings** runs `gh0st keyring prepare` before `passwd`: the daemon
  checks the old password and writes `login.next`, wrapped for the new one.
  After `passwd`, `gh0st keyring commit 1` swaps it in, or `0` drops it.
- **If the device dies in between**, the next unlock with the new password
  finds `login.next` and promotes it. A clean open of the vault deletes a
  leftover one.
- **`passwd` run elsewhere** leaves the vault on the old password. The
  system accepts the new one and the vault refuses it, so the shell asks
  for the old one, once, and the daemon rewraps.
- The private socket for prepare and commit is
  `$XDG_RUNTIME_DIR/gh0st/keyring.sock`, mode 0600. It checks the peer's
  UID, and after a wrong old password it takes no new try for 2 s.

## Hardening

- The daemon is not dumpable and has no core limit. With
  `kernel.yama.ptrace_scope=1` (gh0st-base), no app reads its memory.
- Secrets sit on `mlock`ed pages left out of dumps, zeroed before they go
  back. Message buffers are zeroed as they are read and written.
- A message past 1 MiB is skipped. A call among those is answered
  `LimitsExceeded`, and one that doesn't decode is dropped. Neither stops
  the daemon.
- Sessions and prompts are capped per connection.

## Limits

- An app that isn't isolated (`docs/isolation.md`), the terminal and
  anything run through sudo can do what UID 1000 can: read keys while
  another app is in front, or call the keyring under a name it chooses.
  For them, approval is a speed bump, not a wall.
- Sessions are `plain`. The bus is local and only UID 1000 can listen on
  it, so the DH session would add nothing; libsecret and python
  `secretstorage` fall back to plain.
- A vault made under a password the user later forgets is lost. There is
  no recovery key.

## Tests

- **Unit tests** cover the vault (round trip, wrong password, a changed byte
  anywhere, a cut file, hostile costs, rewrap), the body codec under
  mutation, and the service's flows: approval, lock and unlock, a stale
  password, and password changes.
- **`crates/gh0st-keyringd/tests/compat.sh`** and
  **`crates/gh0st-session/tests/compat.sh`** run real clients under
  `dbus-run-session` in CI: `secret-tool`, `gdbus`, python `keyring` and
  `secretstorage`, `git-credential-libsecret`, `notify-send`.
- The prompt screens are mockup scenes under the design lint.
