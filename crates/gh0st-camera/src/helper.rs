//! Starting the camera helper and talking to it. The helper owns libcamera
//! and the encoders; it is a C++ program (`camd/`) the SDK runs as a
//! sidecar. When the app is paused the helper must still act on the `stop`
//! the app sent: close a recording, let go of the sensor. It exits on
//! `quit`, on a hang-up, and when the app dies.

use crate::proto::{Cmd, Event};
use gh0st_app::frames::{Ring, SCREEN_PX};
use gh0st_app::sidecar::{self, Env, Link, Sidecar};
use std::io;
use std::path::PathBuf;

/// Where the helper finds its end of the link and the ring: fd numbers.
pub const FD_ENV: &str = "GH0ST_CAMD_FD";
pub const RING_ENV: &str = "GH0ST_CAMD_RING";
const ENV: Env = Env {
    link: FD_ENV,
    ring: RING_ENV,
};
/// Overrides the helper's path. `fake` runs this binary's test helper.
pub const PATH_ENV: &str = "GH0ST_CAMD";
pub const DEFAULT_PATH: &str = "/usr/libexec/gh0st-camd";

/// Slots the preview cycles through. The last slot holds thumbnails.
pub const PREVIEW_SLOTS: u32 = 3;
pub const SLOTS: u32 = PREVIEW_SLOTS + 1;
pub const THUMB_SLOT: u32 = PREVIEW_SLOTS;

/// Which helper runs.
#[derive(Clone, Debug)]
pub enum Source {
    /// A program: the real helper, or this binary with `--fake-helper`.
    Exec(Vec<String>),
    /// The fake helper on a thread, for tests and mockups that have no
    /// binary to run. Without a camera it answers as a Lite does.
    Fake { camera: bool },
}

impl Source {
    /// The real helper, or what `GH0ST_CAMD` names.
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

/// The helper as this app speaks to it: the SDK's sidecar with the
/// camera's words.
pub struct Helper {
    side: Sidecar,
}

impl Helper {
    pub fn start(source: &Source) -> io::Result<Helper> {
        let ring = Ring::create(c"gh0st-camera", SLOTS, SCREEN_PX)?;
        let side = match source {
            Source::Exec(argv) => Sidecar::spawn(argv, ENV, ring)?,
            &Source::Fake { camera } => Sidecar::thread(ring, move |link, ring| {
                crate::fake::serve(link, ring, camera)
            })?,
        };
        Ok(Helper { side })
    }

    pub fn ring(&self) -> &Ring {
        &self.side.ring
    }

    /// The child to adopt, when it is a process.
    pub fn pid(&self) -> Option<i32> {
        self.side.pid
    }

    pub fn fd(&self) -> std::os::fd::RawFd {
        self.side.fd()
    }

    /// False when it didn't go: the helper is gone or not reading.
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

    /// True once the helper hung up.
    pub fn closed(&self) -> bool {
        self.side.closed()
    }
}

/// A helper handle dropped without `quit` (a test, a panic) still tells the
/// helper to go: the hang-up alone is lost on datagram links.
impl Drop for Helper {
    fn drop(&mut self) {
        self.send(&Cmd::Quit);
    }
}

/// The helper's side: its link and ring from the env this app set.
pub fn inherited() -> io::Result<(Link, Ring)> {
    sidecar::inherited(ENV)
}

/// Where photos and videos go: stock Camera's folder.
pub fn default_dir() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_else(|| "/tmp".into());
    PathBuf::from(home).join("Pictures/DCIM/Camera")
}
