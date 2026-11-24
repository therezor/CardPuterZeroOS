//! ssh asks for passwords, passphrases and new host keys through an askpass
//! program. Ours is this binary again: it carries ssh's prompt to the
//! worker over a socket in a folder only this user can open, and prints the
//! answer. ssh closes every fd it inherits, so a path is the way through.

use crate::engine::Ask;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Set in ssh's env: the socket to call. Its presence makes this binary the
/// askpass client.
pub const ENV: &str = "GH0ST_FILES_ASKPASS";

pub struct Askpass {
    listener: UnixListener,
    path: PathBuf,
}

impl Askpass {
    pub fn open() -> std::io::Result<Askpass> {
        // SAFETY: getuid has no failure mode.
        let uid = unsafe { libc::getuid() };
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!("gh0st-files-{uid}"));
        // Made 0700 by us, and checked: a link or a stranger's folder
        // planted in /tmp is refused.
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        let _ = std::fs::DirBuilder::new().mode(0o700).create(&dir);
        let m = std::fs::symlink_metadata(&dir)?;
        if !m.is_dir() || m.uid() != uid || m.mode() & 0o077 != 0 {
            return Err(std::io::Error::other("the askpass folder isn't ours alone"));
        }
        let path = dir.join(format!("askpass-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        Ok(Askpass { listener, path })
    }

    pub fn fd(&self) -> RawFd {
        self.listener.as_raw_fd()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A caller and its prompt.
    pub fn accept(&self) -> Option<(UnixStream, String)> {
        let (mut s, _) = self.listener.accept().ok()?;
        s.set_nonblocking(false).ok()?;
        s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        let mut prompt = Vec::new();
        let _ = (&mut s).take(4096).read_to_end(&mut prompt);
        Some((s, String::from_utf8_lossy(&prompt).into_owned()))
    }
}

impl Drop for Askpass {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Answers a caller: the text, or no.
pub fn answer(s: &mut UnixStream, text: Option<&str>) {
    let reply = match text {
        Some(t) => format!("1{t}"),
        None => "0".into(),
    };
    let _ = s.write_all(reply.as_bytes());
}

/// What ssh's prompt asks. Only ssh's own question for our key's
/// passphrase is answered from the keyring; any other prompt that says
/// "passphrase" may be a server's words, and goes to the user.
pub fn classify(prompt: &str, server: &str, retry: bool, key: Option<&Path>) -> Ask {
    if prompt.contains("(yes/no") {
        let host = prompt
            .split_once("authenticity of host '")
            .and_then(|(_, r)| r.split_once('\''))
            .map(|(h, _)| h.split(" (").next().unwrap_or(h).to_string())
            .unwrap_or_else(|| server.to_string());
        let fingerprint = prompt
            .split_whitespace()
            .find(|w| w.starts_with("SHA256:"))
            .map(|w| w.trim_end_matches('.').to_string())
            .unwrap_or_default();
        return Ask::HostKey {
            server: server.into(),
            host,
            fingerprint,
        };
    }
    if let Some(k) = key {
        let ours = format!("Enter passphrase for key '{}'", k.display());
        if prompt.trim_end().trim_end_matches(':') == ours {
            return Ask::Passphrase;
        }
    }
    Ask::Password {
        server: server.into(),
        retry,
    }
}

/// The client: `gh0st-files <prompt>`, as ssh runs it. Prints the answer
/// for ssh; exits 1 when the user said no.
pub fn client(path: &str) -> i32 {
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let Ok(mut s) = UnixStream::connect(path) else {
        return 1;
    };
    if s.write_all(prompt.as_bytes()).is_err() {
        return 1;
    }
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    if s.read_to_string(&mut reply).is_err() {
        return 1;
    }
    match reply.strip_prefix('1') {
        Some(text) => {
            println!("{text}");
            0
        }
        None => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_read_as_questions() {
        let hk = "The authenticity of host '192.168.1.5 (192.168.1.5)' can't be established.\n\
                  ED25519 key fingerprint is SHA256:eV9BNqsF/x+y.\n\
                  Are you sure you want to continue connecting (yes/no/[fingerprint])? ";
        assert_eq!(
            classify(hk, "nas", false, None),
            Ask::HostKey {
                server: "nas".into(),
                host: "192.168.1.5".into(),
                fingerprint: "SHA256:eV9BNqsF/x+y".into()
            }
        );
        assert_eq!(
            classify("me@nas.local's password: ", "nas", true, None),
            Ask::Password {
                server: "nas".into(),
                retry: true
            }
        );
        let key = Path::new("/x/id_ed25519");
        assert_eq!(
            classify(
                "Enter passphrase for key '/x/id_ed25519': ",
                "nas",
                false,
                Some(key)
            ),
            Ask::Passphrase
        );
        // A server's own prompt that says "passphrase" goes to the user.
        assert_eq!(
            classify("(me@nas) Enter your passphrase: ", "nas", false, Some(key)),
            Ask::Password {
                server: "nas".into(),
                retry: false
            }
        );
    }

    #[test]
    fn a_prompt_crosses_and_the_answer_comes_back() {
        let a = Askpass::open().unwrap();
        let path = a.path().to_path_buf();
        let t = std::thread::spawn(move || {
            let mut s = UnixStream::connect(&path).unwrap();
            s.write_all(b"pw? ").unwrap();
            s.shutdown(std::net::Shutdown::Write).unwrap();
            let mut r = String::new();
            s.read_to_string(&mut r).unwrap();
            r
        });
        let got = loop {
            if let Some(x) = a.accept() {
                break x;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let (mut s, prompt) = got;
        assert_eq!(prompt, "pw? ");
        answer(&mut s, Some("hunter2"));
        drop(s);
        assert_eq!(t.join().unwrap(), "1hunter2");
    }
}
