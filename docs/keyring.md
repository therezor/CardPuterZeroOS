# The keyring

`gh0st keyring` is the system keyring. It serves the freedesktop Secret
Service (`org.freedesktop.secrets`) on the session bus, so libsecret,
`secret-tool`, python `keyring`, git's libsecret helper, Chromium and every
other Linux app that keeps secrets use it unchanged.

`gh0st session` runs beside it with four more session services:

- `org.freedesktop.Notifications`: a notification becomes a notice file the
  shell toasts (`docs/apps.md`, Notifications). An app keeps eight at most.
- `org.freedesktop.ScreenSaver` Inhibit and UnInhibit: the app's `awake`, as
  if it had said it on its link. It ends with the last inhibitor or the
  connection. A program outside apps is named by its pid, and keeps awake
  the app whose process group it is in.
- The XDG portal (`docs/portal.md`).
- `org.gh0st.Clipboard`: one clipboard for every app (`docs/apps.md`,
  Session services).

The shell starts both on socket pairs before any app, and starts one again
the moment it dies, so no app gets time to take its bus names. A dead shell
takes them with it. A daemon that died takes its questions with it, and the
other's stay. A new `gh0st session` numbers its requests afresh: the shell
drops the old one's questions, closes its choosers and lets go of its
inhibitors.

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
- Argon2id costs are in the header: 32 MiB and two passes, 0.5 s on the
  device. Anyone with the card can guess the same password against
  `/etc/shadow` at about 0.3 s a try, so more would only slow the user. A
  vault under other costs is wrapped again under these when it opens. A
  file asking for more than 256 MiB or 16 passes is refused before any
  work.
- The crypto is RustCrypto's `argon2` and `chacha20poly1305`, with `zeroize`.

## Unlock and lock

- The lock screen runs `unix_chkpwd`. When it says the password is right,
  the shell sends it to the keyring over its socket pair, never a bus.
- The vault opens on its own tag. The shell hands it the password as
  `unix_chkpwd` starts, so both run side by side; that early try opens an
  existing vault only. Only a checked password can make a new vault, and
  one the system refuses at the lock screen locks it again.
- A device lock closes the vault: the data key and every secret go, except
  NetworkManager's Wi-Fi passwords. None are stored yet: there is no
  NetworkManager secret agent.
- Attributes and labels stay in memory after the first unlock. A search
  while locked still finds the right items, reported locked, and the client
  asks to unlock them.
- With the lock password off, the shell asks for the password once after
  the unlock. Until something opens the vault, a search or lookup answers
  `IsLocked`, never "not found", so no app stores a second copy.
- After Lock after in the dark, the vault closes then, not at the key that
  wakes the lock screen.

## Who is asking

- A process in a cgroup `app-<desktop stem>` right under `gh0st.service` is
  that app. The shell puts no app in such a cgroup yet, so every caller is
  its executable's path.
- The pid comes from the bus, with a pidfd where dbus-daemon gives one, so a
  reused pid can't pass for another program.
- An item belongs to the app that stored it. To any other app it looks
  locked, `GetSecret` says `IsLocked`, and `Unlock` asks the user through
  the shell:
  - 5 allows once, for this connection.
  - 7 allows always; the vault keeps it.
  - Esc says no.
- A new question takes no answer for its first 600 ms, and Enter answers
  nothing: a key meant for the app can't approve what the app asked for.
- Labels and attributes are not secret: once the vault has opened, any app
  can list them, as with gnome-keyring. Only the secret is guarded.

## Prompts

- A Secret Service prompt (`Unlock`, `Delete`, `CreateCollection`) asks on
  a shell screen while the shell holds `EVIOCGRAB`. The app in front is
  parked first, as Fn+K parks it.
- An app that can't be parked (run through sudo) or that holds the keyboard
  could read the answer, so the question is refused instead.
- A prompt's password goes through `unix_chkpwd` and the lock's backoff
  before the vault sees it.
- While the device is locked, prompts wait for the unlock.

## The Keyring app

`gh0st-keyring` is a plain client of the same API, plus `org.gh0st.Keyring`
for what the standard lacks. Nothing there returns a secret.

- **Home:** Passwords, Wi-Fi and App access with their counts. Lock on 5
  and Add on 7 while the keyring is open; Unlock on 6 while it is locked.
- **Passwords and Wi-Fi:** items by the app that stored them.
- **An item:** who stored it, and the secret behind dots. Show on 5 asks
  the shell for the account password, even with the keyring open; Delete
  on 7 confirms, then goes through the keyring's own prompt.
- **App access:** which app may read which other app's secret. Revoke on 6
  makes the app ask again.
- **Add:** a name and a password, stored as the app's own
  (`org.gh0st.Password`).

The app makes quick calls that answer at once and waits on no prompt: a
prompt's end comes on the bus fd, which the page watches.

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

- The daemon and the shell are not dumpable, so only root can trace them or
  read their memory. The kernel has no Yama; where one does,
  `kernel.yama.ptrace_scope=1` (gh0st-base) narrows tracing further.
- Secrets sit on `mlock`ed pages left out of dumps, zeroed before they go
  back. Message buffers are zeroed as they are read and written.
- A message past 1 MiB is skipped. A call among those is answered
  `LimitsExceeded`, and one that doesn't decode is dropped. Neither stops
  the daemon.
- Sessions and prompts are capped per connection.

## Limits

- Every app, the terminal and anything run through sudo can do what UID
  1000 can: read keys while another app is in front, or call the keyring
  under a name it chooses. For them, approval is a speed bump, not a wall.
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
- **`crates/gh0st/tests/keyring_sim.sh`** runs the real shell under
  `--sim` with both daemons: unlock from the lock screen, approve once, a
  key too soon, deny, an app in front parked and brought back, a
  question after the dark over an app, and `gh0st open` from the app in
  front opening with no key pressed.
- **`crates/gh0st-keyringd/tests/compat.sh`** and
  **`crates/gh0st-session/tests/compat.sh`** run real clients under
  `dbus-run-session` in CI: `secret-tool`, `gdbus`, python `keyring` and
  `secretstorage`, `git-credential-libsecret`, `notify-send`.
- The prompt screens are mockup scenes under the design lint.
