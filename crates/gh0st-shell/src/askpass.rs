//! The shell's end of sudo for root apps (docs/isolation.md, Root apps):
//! a listener only the device user reaches, and the helpers waiting on it.
//! A helper counts when it is our own binary, run by a real sudo, in the
//! cgroup of an app. Which apps may ask is the shell's call, by stem.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;

use gh0st_core::askpass;

/// Helpers waiting at once. More are turned away.
const MAX_CONNS: usize = 4;

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// App `stem`'s sudo wants the password: ask the user, then `answer`.
    Ask { id: u64, stem: String },
    /// Its sudo went away: drop the question.
    Gone(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Waiting for `ask`.
    Asked,
    /// Sent `go`, waiting for `ready`.
    Going,
    /// The user is being asked.
    Waiting,
}

struct Conn {
    id: u64,
    fd: OwnedFd,
    pid: u32,
    stem: String,
    step: Step,
}

pub struct Server {
    listener: OwnedFd,
    conns: Vec<Conn>,
    next: u64,
    uid: u32,
    /// A sim's stand-in: every helper of the user asks for this app. A sim
    /// has no sudo and no cgroups to check.
    sim_app: Option<String>,
}

impl Server {
    pub fn open(runtime: &Path, sim_app: Option<String>) -> std::io::Result<Server> {
        Ok(Server {
            listener: askpass::listen(&askpass::socket_path(runtime))?,
            conns: Vec::new(),
            next: 1,
            // SAFETY: getuid has no failure mode.
            uid: unsafe { libc::getuid() },
            sim_app,
        })
    }

    /// The listener, then each helper, for poll.
    pub fn fds(&self) -> Vec<RawFd> {
        std::iter::once(self.listener.as_raw_fd())
            .chain(self.conns.iter().map(|c| c.fd.as_raw_fd()))
            .collect()
    }

    /// After poll: `ready[i]` says whether fd `i` of [`fds`](Server::fds)
    /// was readable then.
    pub fn ready(&mut self, ready: &[bool]) -> Vec<Event> {
        let mut out = Vec::new();
        let mut gone = Vec::new();
        for (i, c) in self.conns.iter_mut().enumerate() {
            if !ready.get(i + 1).copied().unwrap_or(false) {
                continue;
            }
            let mut buf = [0u8; askpass::MAX_MSG];
            let n = match askpass::recv(&c.fd, &mut buf) {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(_) => 0,
            };
            let msg = &buf[..n];
            match (c.step, msg) {
                (_, []) => gone.push(c.id),
                (Step::Asked, m) if m.starts_with(b"ask\t") => {
                    if askpass::send(&c.fd, askpass::GO) {
                        c.step = Step::Going;
                    } else {
                        gone.push(c.id);
                    }
                }
                (Step::Going, askpass::READY) => {
                    c.step = Step::Waiting;
                    out.push(Event::Ask {
                        id: c.id,
                        stem: c.stem.clone(),
                    });
                }
                _ => {
                    askpass::send(&c.fd, askpass::NO);
                    gone.push(c.id);
                }
            }
        }
        for id in gone {
            if let Some(at) = self.conns.iter().position(|c| c.id == id) {
                let c = self.conns.remove(at);
                if c.step == Step::Waiting {
                    out.push(Event::Gone(id));
                }
            }
        }
        if ready.first().copied().unwrap_or(false) {
            self.accept();
        }
        out
    }

    /// Takes every waiting connection. One that isn't our helper, run by a
    /// real sudo, in an app's cgroup, is closed at once; so is a second one
    /// from the same app.
    fn accept(&mut self) {
        loop {
            #[cfg(target_os = "linux")]
            // SAFETY: accept4 on our listener; the new fd is ours alone.
            let fd = unsafe {
                libc::accept4(
                    self.listener.as_raw_fd(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                )
            };
            #[cfg(not(target_os = "linux"))]
            let fd = -1;
            if fd < 0 {
                return;
            }
            // SAFETY: the fd is new and ours alone.
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            let Some(pid) = peer(&fd, self.uid) else {
                continue;
            };
            let Some(stem) = self.sim_app.clone().or_else(|| who(pid, self.uid)) else {
                eprintln!("gh0st shell: askpass: refused pid {pid}");
                continue;
            };
            if self.conns.len() >= MAX_CONNS || self.conns.iter().any(|c| c.stem == stem) {
                askpass::send(&fd, askpass::NO);
                continue;
            }
            let id = self.next;
            self.next += 1;
            self.conns.push(Conn {
                id,
                fd,
                pid,
                stem,
                step: Step::Asked,
            });
        }
    }

    /// The user's answer for `id`: the password, or no. The helper goes.
    pub fn answer(&mut self, id: u64, pw: Option<&[u8]>) {
        let Some(at) = self.conns.iter().position(|c| c.id == id) else {
            return;
        };
        let c = self.conns.remove(at);
        match pw {
            Some(pw) => {
                let mut m = askpass::ok(pw);
                askpass::send(&c.fd, &m);
                gh0st_core::password::wipe(&mut m);
            }
            None => {
                askpass::send(&c.fd, askpass::NO);
            }
        }
        eprintln!(
            "gh0st shell: askpass: {} for {}",
            if pw.is_some() { "answered" } else { "refused" },
            c.pid
        );
    }

    /// Every helper waiting on a question `asked` no longer holds hears no:
    /// none waits for an answer that will never come.
    pub fn reconcile(&mut self, asked: impl Fn(u64) -> bool) {
        let lost: Vec<u64> = self
            .conns
            .iter()
            .filter(|c| c.step == Step::Waiting && !asked(c.id))
            .map(|c| c.id)
            .collect();
        for id in lost {
            self.answer(id, None);
        }
    }
}

/// The peer's pid, when it is the device user's.
fn peer(fd: &OwnedFd, uid: u32) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: a zeroed ucred is valid.
        let mut c: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: getsockopt fills a ucred of the length we pass.
        let r = unsafe {
            libc::getsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut c as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        (r == 0 && c.uid == uid && c.pid > 0).then_some(c.pid as u32)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (fd, uid);
        None
    }
}

/// The app whose sudo runs helper `pid`, from `/proc`.
fn who(pid: u32, uid: u32) -> Option<String> {
    check(Path::new("/proc"), Path::new(askpass::HELPER), pid, uid)
}

/// The app whose sudo runs helper `pid`, read under `proc`: the helper is
/// the same file as `helper`, so an app can't point sudo at a program of
/// its own and be handed the password; its parent is a real sudo of
/// `uid`; and it runs in an app's cgroup, which names the app.
fn check(proc: &Path, helper: &Path, pid: u32, uid: u32) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let dir = proc.join(pid.to_string());
    let exe = std::fs::metadata(dir.join("exe")).ok()?;
    let ours = std::fs::metadata(helper).ok()?;
    if (exe.dev(), exe.ino()) != (ours.dev(), ours.ino()) {
        return None;
    }
    let parent = askpass::ppid(&std::fs::read_to_string(dir.join("stat")).ok()?)?;
    let status = std::fs::read_to_string(proc.join(parent.to_string()).join("status")).ok()?;
    if !askpass::is_sudo_of(&status, uid) {
        return None;
    }
    gh0st_core::ident::app_of_cgroup(&std::fs::read_to_string(dir.join("cgroup")).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `/proc` with helper 300, its parent 200, and a stand-in binary.
    fn fixture(
        name: &str,
        parent_status: &str,
        cgroup: &str,
        exe_is_ours: bool,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("gh0st-askpass-proc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let helper = root.join("gh0st-askpass");
        let other = root.join("evil");
        std::fs::create_dir_all(root.join("proc/300")).unwrap();
        std::fs::create_dir_all(root.join("proc/200")).unwrap();
        std::fs::write(&helper, b"ours").unwrap();
        std::fs::write(&other, b"theirs").unwrap();
        let target = if exe_is_ours { &helper } else { &other };
        std::os::unix::fs::symlink(target, root.join("proc/300/exe")).unwrap();
        std::fs::write(
            root.join("proc/300/stat"),
            "300 (gh0st-askpass) S 200 300 100 0 -1",
        )
        .unwrap();
        std::fs::write(root.join("proc/300/cgroup"), cgroup).unwrap();
        std::fs::write(root.join("proc/200/status"), parent_status).unwrap();
        (root, helper)
    }

    const SUDO: &str = "Name:\tsudo\nUid:\t1000\t0\t0\t0\n";
    const CG: &str =
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/gh0st.service/app-raspyjack\n";

    #[test]
    fn our_helper_under_a_real_sudo_names_its_app() {
        let (root, helper) = fixture("ok", SUDO, CG, true);
        assert_eq!(
            check(&root.join("proc"), &helper, 300, 1000).as_deref(),
            Some("raspyjack")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn anything_else_is_refused() {
        // An app's own program in sudo's place would be handed the password.
        let (root, helper) = fixture("exe", SUDO, CG, false);
        assert_eq!(check(&root.join("proc"), &helper, 300, 1000), None);
        let _ = std::fs::remove_dir_all(&root);
        // Not run by a root sudo: a sandboxed app, or a fake parent.
        let (root, helper) = fixture(
            "parent",
            "Name:\tsudo\nUid:\t1000\t1000\t1000\t1000\n",
            CG,
            true,
        );
        assert_eq!(check(&root.join("proc"), &helper, 300, 1000), None);
        let _ = std::fs::remove_dir_all(&root);
        // Outside any app's cgroup: no app to name, so no one to ask for.
        let (root, helper) = fixture("cgroup", SUDO, "0::/user.slice/session-3.scope\n", true);
        assert_eq!(check(&root.join("proc"), &helper, 300, 1000), None);
        let _ = std::fs::remove_dir_all(&root);
        // Another user's sudo.
        let (root, helper) = fixture("uid", SUDO, CG, true);
        assert_eq!(check(&root.join("proc"), &helper, 300, 1001), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
