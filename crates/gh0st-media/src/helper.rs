//! Starting `gh0st-mediad` and talking to it, as an SDK sidecar.

use crate::proto::{Cmd, Event};
use gh0st_app::frames::{Ring, SCREEN_PX};
use gh0st_app::sidecar::{self, Env, Link, Sidecar};
use std::io;
use std::os::fd::RawFd;

pub const FD_ENV: &str = "GH0ST_MEDIAD_FD";
pub const RING_ENV: &str = "GH0ST_MEDIAD_RING";
const ENV: Env = Env {
    link: FD_ENV,
    ring: RING_ENV,
};
/// Overrides the helper's path. `fake` runs this binary's test helper.
pub const PATH_ENV: &str = "GH0ST_MEDIAD";
pub const DEFAULT_PATH: &str = "/usr/libexec/gh0st-mediad";

/// Slots a playing video's pictures take in turn; the last answers asks.
pub const PLAY_SLOTS: u32 = 3;
pub const SLOTS: u32 = PLAY_SLOTS + 1;
pub const ASK_SLOT: u32 = PLAY_SLOTS;

/// Which helper runs.
#[derive(Clone, Debug)]
pub enum Source {
    /// A program: the real helper, or this binary with `--fake-helper`.
    Exec(Vec<String>),
    /// The fake helper on a thread, for tests and mockups.
    Fake,
    /// None at all: the helper is missing.
    Missing,
}

impl Source {
    /// The real helper, or what `GH0ST_MEDIAD` names.
    pub fn from_env() -> Source {
        match std::env::var(PATH_ENV).ok().as_deref() {
            Some("fake") => {
                let me = std::env::current_exe()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Source::Exec(vec![me, "--fake-helper".into()])
            }
            Some(p) if !p.is_empty() => Source::Exec(vec![p.into()]),
            _ => Source::Exec(vec![DEFAULT_PATH.into()]),
        }
    }
}

pub struct Helper {
    side: Sidecar,
}

impl Helper {
    pub fn start(source: &Source) -> io::Result<Helper> {
        let ring = Ring::create(c"gh0st-media", SLOTS, SCREEN_PX)?;
        let side = match source {
            Source::Exec(argv) => Sidecar::spawn(argv, ENV, ring)?,
            Source::Fake => Sidecar::thread(ring, crate::fake::serve)?,
            Source::Missing => return Err(io::Error::from(io::ErrorKind::NotFound)),
        };
        Ok(Helper { side })
    }

    pub fn ring(&self) -> &Ring {
        &self.side.ring
    }

    pub fn pid(&self) -> Option<i32> {
        self.side.pid
    }

    pub fn fd(&self) -> RawFd {
        self.side.fd()
    }

    pub fn send(&self, cmd: &Cmd) -> bool {
        let f = cmd.fields();
        let refs: Vec<&str> = f.iter().map(String::as_str).collect();
        self.side.send(&refs)
    }

    /// The next event, or None when nothing waits. Unknown words are
    /// skipped.
    pub fn recv(&self) -> Option<Event> {
        loop {
            let f = self.side.recv()?;
            if let Some(e) = Event::parse(&f) {
                return Some(e);
            }
        }
    }

    pub fn closed(&self) -> bool {
        self.side.closed()
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        self.send(&Cmd::Quit);
    }
}

/// The helper's side: its link and ring from the env this app set.
pub fn inherited() -> io::Result<(Link, Ring)> {
    sidecar::inherited(ENV)
}
