//! The link between the shell and the keyring daemon: one packet per
//! message on a socket pair the shell made, fields split by NUL so a
//! password can hold any byte but NUL (as `unix_chkpwd` takes it).
//! Passwords cross only here, never a bus.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};

/// Where the daemon finds its end.
pub const ENV: &str = "GH0ST_KEYRING_FD";
/// The longest message: a password and a prompt's reason fit easily.
pub const MAX: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Once,
    Always,
    Deny,
}

/// Shell to daemon.
#[derive(Debug, PartialEq, Eq)]
pub enum ToDaemon {
    /// A password `unix_chkpwd` agreed to, from the lock screen or a
    /// prompt. A vault that refuses it was left on an old one.
    Unlock {
        pw: Vec<u8>,
    },
    Lock,
    /// The old password, to rewrap the vault with the held new one.
    Old {
        pw: Vec<u8>,
    },
    Password {
        id: u64,
        pw: Vec<u8>,
    },
    Answer {
        id: u64,
        answer: Answer,
    },
}

/// Daemon to shell.
#[derive(Debug, PartialEq, Eq)]
pub enum ToShell {
    /// Names owned: apps may start.
    Ready,
    Unlocked,
    Wrong,
    /// The device password is right, the vault's is old: ask for it.
    Stale,
    Broken,
    AskPassword {
        id: u64,
        reason: String,
        retry: bool,
    },
    AskApprove {
        id: u64,
        app: String,
        what: String,
    },
    Cancel {
        id: u64,
    },
}

fn join(fields: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(0);
        }
        out.extend_from_slice(f);
    }
    out
}

fn answer_word(a: Answer) -> &'static [u8] {
    match a {
        Answer::Once => b"once",
        Answer::Always => b"always",
        Answer::Deny => b"deny",
    }
}

impl ToDaemon {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ToDaemon::Unlock { pw } => join(&[b"unlock", pw]),
            ToDaemon::Lock => b"lock".to_vec(),
            ToDaemon::Old { pw } => join(&[b"old", pw]),
            ToDaemon::Password { id, pw } => join(&[b"password", id.to_string().as_bytes(), pw]),
            ToDaemon::Answer { id, answer } => {
                join(&[b"answer", id.to_string().as_bytes(), answer_word(*answer)])
            }
        }
    }

    pub fn decode(b: &[u8]) -> Option<ToDaemon> {
        let f: Vec<&[u8]> = b.split(|&c| c == 0).collect();
        let id = |i: usize| -> Option<u64> { std::str::from_utf8(f.get(i)?).ok()?.parse().ok() };
        Some(match (f.first().copied()?, f.len()) {
            (b"unlock", 2) => ToDaemon::Unlock { pw: f[1].to_vec() },
            (b"lock", 1) => ToDaemon::Lock,
            (b"old", 2) => ToDaemon::Old { pw: f[1].to_vec() },
            (b"password", 3) => ToDaemon::Password {
                id: id(1)?,
                pw: f[2].to_vec(),
            },
            (b"answer", 3) => ToDaemon::Answer {
                id: id(1)?,
                answer: match f[2] {
                    b"once" => Answer::Once,
                    b"always" => Answer::Always,
                    _ => Answer::Deny,
                },
            },
            _ => return None,
        })
    }

    /// Zeroes any password it holds.
    pub fn wipe(&mut self) {
        match self {
            ToDaemon::Unlock { pw, .. } | ToDaemon::Old { pw } | ToDaemon::Password { pw, .. } => {
                crate::password::wipe(pw)
            }
            _ => {}
        }
    }
}

impl ToShell {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            ToShell::Ready => b"ready".to_vec(),
            ToShell::Unlocked => b"unlocked".to_vec(),
            ToShell::Wrong => b"wrong".to_vec(),
            ToShell::Stale => b"stale".to_vec(),
            ToShell::Broken => b"broken".to_vec(),
            ToShell::AskPassword { id, reason, retry } => join(&[
                b"ask-password",
                id.to_string().as_bytes(),
                reason.as_bytes(),
                if *retry { b"1" } else { b"0" },
            ]),
            ToShell::AskApprove { id, app, what } => join(&[
                b"ask-approve",
                id.to_string().as_bytes(),
                app.as_bytes(),
                what.as_bytes(),
            ]),
            ToShell::Cancel { id } => join(&[b"cancel", id.to_string().as_bytes()]),
        }
    }

    pub fn decode(b: &[u8]) -> Option<ToShell> {
        let f: Vec<&[u8]> = b.split(|&c| c == 0).collect();
        let id = |i: usize| -> Option<u64> { std::str::from_utf8(f.get(i)?).ok()?.parse().ok() };
        let text = |i: usize| String::from_utf8_lossy(f[i]).into_owned();
        Some(match (f.first().copied()?, f.len()) {
            (b"ready", 1) => ToShell::Ready,
            (b"unlocked", 1) => ToShell::Unlocked,
            (b"wrong", 1) => ToShell::Wrong,
            (b"stale", 1) => ToShell::Stale,
            (b"broken", 1) => ToShell::Broken,
            (b"ask-password", 4) => ToShell::AskPassword {
                id: id(1)?,
                reason: text(2),
                retry: f[3] == b"1",
            },
            (b"ask-approve", 4) => ToShell::AskApprove {
                id: id(1)?,
                app: text(2),
                what: text(3),
            },
            (b"cancel", 2) => ToShell::Cancel { id: id(1)? },
            _ => return None,
        })
    }
}

/// Sends one packet without blocking. The bytes are the caller's to wipe.
pub fn send(fd: RawFd, bytes: &[u8]) -> bool {
    if bytes.len() > MAX {
        return false;
    }
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT;
    #[cfg(not(target_os = "linux"))]
    let flags = libc::MSG_DONTWAIT;
    // SAFETY: sends from a live buffer.
    unsafe { libc::send(fd, bytes.as_ptr().cast(), bytes.len(), flags) >= 0 }
}

pub enum Recv {
    Packet(Vec<u8>),
    Empty,
    Closed,
}

/// One packet, if one waits. The stack copy is zeroed.
pub fn recv(fd: RawFd) -> Recv {
    let mut buf = [0u8; MAX];
    // SAFETY: receives into a stack buffer of its own length.
    let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), libc::MSG_DONTWAIT) };
    let out = if n < 0 {
        match io::Error::last_os_error().raw_os_error() {
            Some(libc::EAGAIN | libc::EINTR) => Recv::Empty,
            _ => Recv::Closed,
        }
    } else if n == 0 {
        Recv::Closed
    } else {
        Recv::Packet(buf[..n as usize].to_vec())
    };
    crate::password::wipe(&mut buf);
    out
}

/// The pair: the shell's end, and the daemon's for the child to inherit.
pub fn pair() -> io::Result<(OwnedFd, OwnedFd)> {
    let (a, b) = crate::link::raw_pair()?;
    crate::rt::set_nonblock_cloexec(a.as_raw_fd())?;
    Ok((a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        for m in [
            ToDaemon::Unlock {
                pw: b"p\tw d".to_vec(),
            },
            ToDaemon::Lock,
            ToDaemon::Old {
                pw: b"old".to_vec(),
            },
            ToDaemon::Password { id: 7, pw: vec![] },
            ToDaemon::Answer {
                id: 3,
                answer: Answer::Always,
            },
        ] {
            assert_eq!(ToDaemon::decode(&m.encode()), Some(m));
        }
        for m in [
            ToShell::Ready,
            ToShell::Stale,
            ToShell::AskPassword {
                id: 1,
                reason: "Chromium wants the keyring".into(),
                retry: true,
            },
            ToShell::AskApprove {
                id: 2,
                app: "Chromium".into(),
                what: "read \u{201c}github\u{201d}".into(),
            },
            ToShell::Cancel { id: 9 },
        ] {
            assert_eq!(ToShell::decode(&m.encode()), Some(m));
        }
        assert_eq!(ToDaemon::decode(b"unlock\0pw\0x"), None);
        assert_eq!(ToDaemon::decode(b"password\0x\0pw"), None);
        assert_eq!(ToShell::decode(b""), None);
    }

    #[test]
    fn packets_cross_the_pair() {
        let (a, b) = pair().unwrap();
        crate::rt::set_nonblock_cloexec(b.as_raw_fd()).unwrap();
        assert!(send(a.as_raw_fd(), &ToShell::Ready.encode()));
        let Recv::Packet(p) = recv(b.as_raw_fd()) else {
            panic!()
        };
        assert_eq!(ToShell::decode(&p), Some(ToShell::Ready));
        assert!(matches!(recv(b.as_raw_fd()), Recv::Empty));
        drop(a);
        assert!(matches!(recv(b.as_raw_fd()), Recv::Closed));
    }
}
