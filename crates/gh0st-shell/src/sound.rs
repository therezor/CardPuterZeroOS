//! The shell's player for system sounds (`gh0st_core::sound`). One `pacat`
//! child takes raw samples on a pipe. It starts with a sound, or as the
//! screen wakes, and ends `HOLD_MS` after the last sound, or when the
//! screen goes dark. An open stream keeps PipeWire's graph and the codec
//! running, about 2,000 wakeups a second even when silent, so it is held
//! only while the user is busy. A write never blocks the shell: a sound
//! that doesn't fit is dropped, and a dead player starts again at the next
//! sound.
//!
//! Measured on the device: a held stream plays a sound at once; a new one
//! takes 100 to 190 ms, so one `pacat` per sound would lag every key. The
//! amplifier powers down about 10 s after the last stream ends. A stream
//! that starts after that loses most of its first sound unless silence
//! comes first: 50 ms of it was enough.

use gh0st_core::sound::{RATE, Sound};
use std::collections::HashMap;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::process::{Child, ChildStdin, Command, Stdio};

/// How long the stream stays open after a sound: keys closer than this
/// sound at once.
const HOLD_MS: u64 = 2000;
/// After a player that would not start, how long before trying again.
const RETRY_MS: u64 = 30_000;
/// A player that exits sooner than this after its start failed to start:
/// no sound server, say. Each key would start another.
const SHORT_LIFE_MS: u64 = 1000;
/// Sounds queue behind each other; past this much waiting, one is dropped
/// rather than lag the keys.
const QUEUE_MS: u64 = 250;
/// The amplifier is off this long after a stream ends: PipeWire suspends
/// the sink after 5 s, and the codec's power-down follows 5 s later.
/// Measured at 10 s; less, so a doubt costs a lead, not a sound.
const AMP_OFF_MS: u64 = 9_000;
/// Silence ahead of the first sound while the amplifier starts.
const LEAD_MS: u64 = 100;
/// The least a warm-up writes: the stream's latency, so it starts.
const START_MS: u64 = 30;
/// Room in the pipe for the longest sound and a little queue.
#[cfg(target_os = "linux")]
const PIPE_BYTES: libc::c_int = 256 * 1024;

pub struct Player {
    /// `pacat`, a test's stub, or None: a sim plays nothing.
    program: Option<String>,
    child: Option<(Child, ChildStdin)>,
    started_at: u64,
    close_at: Option<u64>,
    failed_at: Option<u64>,
    /// When what was written so far has played.
    busy_until: u64,
    /// When the last stream's sound ended. None: never, or unknown.
    ended_at: Option<u64>,
    /// Each sound as bytes, made on first use.
    made: HashMap<Sound, Vec<u8>>,
}

impl Player {
    pub fn new(program: Option<String>) -> Player {
        Player {
            program,
            child: None,
            started_at: 0,
            close_at: None,
            failed_at: None,
            busy_until: 0,
            ended_at: None,
            made: HashMap::new(),
        }
    }

    pub fn play(&mut self, s: Sound, now: u64) {
        let start = self.busy_until.max(now);
        // A tick that would land on another sound adds nothing.
        if (start > now && s == Sound::Move) || start > now + QUEUE_MS {
            return;
        }
        let lead = self.lead_ms(now);
        if !self.open(now) {
            return;
        }
        let bytes = self
            .made
            .entry(s)
            .or_insert_with(|| s.render().iter().flat_map(|v| v.to_le_bytes()).collect());
        let ms = lead + bytes.len() as u64 * 1000 / (RATE as u64 * 2);
        let Some((_, stdin)) = &mut self.child else {
            return;
        };
        let wrote = if lead > 0 {
            let mut all = silence(lead);
            all.extend_from_slice(bytes);
            stdin.write(&all)
        } else {
            stdin.write(bytes)
        };
        self.wrote(wrote, start, ms, now);
    }

    /// Opens the stream ahead of a sound that may come, as the screen
    /// wakes: the amplifier is up by the first key. Silence starts it.
    pub fn warm(&mut self, now: u64) {
        if self.child.is_some() {
            self.close_at = self.close_at.map(|t| t.max(now + HOLD_MS));
            return;
        }
        let ms = self.lead_ms(now).max(START_MS);
        if !self.open(now) {
            return;
        }
        let Some((_, stdin)) = &mut self.child else {
            return;
        };
        let wrote = stdin.write(&silence(ms));
        self.wrote(wrote, now, ms, now);
    }

    /// The silence a new stream needs first: none while the amplifier is
    /// still on from the last one.
    fn lead_ms(&self, now: u64) -> u64 {
        let cold = self.ended_at.is_none_or(|t| now >= t + AMP_OFF_MS);
        if self.child.is_none() && cold {
            LEAD_MS
        } else {
            0
        }
    }

    fn wrote(&mut self, r: std::io::Result<usize>, start: u64, ms: u64, now: u64) {
        match r {
            Ok(_) => {
                self.busy_until = start + ms;
                self.close_at = Some(self.busy_until + HOLD_MS);
            }
            // Full: this sound is dropped, the next may fit.
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => self.lost(now),
        }
    }

    fn open(&mut self, now: u64) -> bool {
        if self.child.is_some() {
            return true;
        }
        let Some(program) = &self.program else {
            return false;
        };
        if self.failed_at.is_some_and(|t| now < t + RETRY_MS) {
            return false;
        }
        let rate = format!("--rate={RATE}");
        let spawned = Command::new(program)
            .args([
                "--playback",
                "--raw",
                "--format=s16le",
                "--channels=1",
                &rate,
                "--latency-msec=30",
                "--client-name=gh0st",
                "--stream-name=System sounds",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = spawned else {
            self.failed_at = Some(now);
            return false;
        };
        let Some(stdin) = child.stdin.take() else {
            return false;
        };
        let fd = stdin.as_raw_fd();
        let _ = gh0st_core::rt::set_nonblock_cloexec(fd);
        #[cfg(target_os = "linux")]
        // SAFETY: resizing a pipe we own; a failure leaves the default.
        unsafe {
            libc::fcntl(fd, libc::F_SETPIPE_SZ, PIPE_BYTES);
        }
        self.child = Some((child, stdin));
        self.started_at = now;
        self.busy_until = now;
        true
    }

    /// Ends the stream: `pacat` plays what it holds and exits, and the
    /// shell's reaper collects it.
    pub fn close(&mut self, now: u64) {
        if self.child.take().is_some() {
            self.ended_at = Some(self.busy_until.max(now));
        }
        self.close_at = None;
        self.busy_until = 0;
    }

    /// The player died under us. What the amplifier does is unknown, so
    /// the next stream starts as a cold one.
    fn lost(&mut self, now: u64) {
        if now < self.started_at + SHORT_LIFE_MS {
            self.failed_at = Some(now);
        }
        self.close(now);
        self.ended_at = None;
    }

    /// When the stream closes if no sound comes first.
    pub fn deadline(&self) -> Option<u64> {
        self.close_at
    }

    pub fn tick(&mut self, now: u64) {
        if self.close_at.is_some_and(|t| now >= t) {
            self.close(now);
        }
    }

    /// The reaper got `pid` at `now`: true if it was the player.
    pub fn exited(&mut self, pid: i32, now: u64) -> bool {
        let ours = self
            .child
            .as_ref()
            .is_some_and(|(c, _)| c.id() as i32 == pid);
        if ours {
            self.lost(now);
        }
        ours
    }
}

fn silence(ms: u64) -> Vec<u8> {
    vec![0; (ms * RATE as u64 / 1000) as usize * 2]
}

/// True once per boot: the first shell since power on plays the boot
/// chime, a restarted one doesn't. The mark lives in the runtime folder,
/// which is gone at power off.
pub fn first_since_boot() -> bool {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return false;
    };
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(std::path::Path::new(&dir).join("gh0st-booted"))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A player whose `pacat` writes what it gets to a file.
    fn stub(name: &str) -> (Player, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("gh0st-player-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("out");
        let prog = dir.join("pacat");
        std::fs::write(
            &prog,
            format!("#!/bin/sh\nexec cat > '{}'\n", out.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&prog, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A test thread that forks while the script is being written holds
        // it open until it execs, and running it then fails (ETXTBSY).
        for _ in 0..100 {
            match std::process::Command::new(&prog)
                .stdin(std::process::Stdio::null())
                .status()
            {
                Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                _ => break,
            }
        }
        (Player::new(Some(prog.display().to_string())), out)
    }

    fn played(p: &mut Player, out: &std::path::Path) -> Vec<u8> {
        let mut child = p.child.take().map(|(c, _)| c).unwrap();
        child.wait().unwrap();
        std::fs::read(out).unwrap()
    }

    fn bytes(s: Sound) -> usize {
        s.render().len() * 2
    }

    fn ms(s: Sound) -> u64 {
        bytes(s) as u64 * 1000 / (RATE as u64 * 2)
    }

    const LEAD: usize = (LEAD_MS * RATE as u64 / 1000) as usize * 2;

    #[test]
    fn a_sound_opens_the_stream_and_the_hold_closes_it() {
        let (mut p, out) = stub("hold");
        p.play(Sound::Select, 1000);
        let end = p.deadline().unwrap();
        assert_eq!(end, 1000 + LEAD_MS + ms(Sound::Select) + HOLD_MS);
        p.tick(end - 1);
        assert!(p.child.is_some());
        // The first stream since start is a cold one: silence, then the sound.
        let pcm = played(&mut p, &out);
        assert_eq!(pcm.len(), LEAD + bytes(Sound::Select));
        assert!(pcm[..LEAD].iter().all(|&b| b == 0));
        assert!(pcm[LEAD..].iter().any(|&b| b != 0));
        p.tick(end);
        assert_eq!(p.deadline(), None);
    }

    #[test]
    fn ticks_that_would_overlap_are_dropped_and_a_queue_stays_short() {
        let (mut p, out) = stub("queue");
        p.play(Sound::Notice, 0);
        let notice_end = LEAD_MS + ms(Sound::Notice);
        // Would wait most of the notice: past QUEUE_MS.
        p.play(Sound::Boot, 10);
        // Waits its turn, near the notice's end.
        p.play(Sound::Select, notice_end - 100);
        p.play(Sound::Move, notice_end - 90);
        let want = LEAD + bytes(Sound::Notice) + bytes(Sound::Select);
        assert_eq!(played(&mut p, &out).len(), want);
    }

    #[test]
    fn no_program_plays_nothing_and_a_missing_one_waits_to_retry() {
        let mut none = Player::new(None);
        none.play(Sound::Select, 0);
        assert!(none.child.is_none() && none.deadline().is_none());
        let mut gone = Player::new(Some("/nonexistent/pacat".into()));
        gone.play(Sound::Select, 0);
        assert_eq!(gone.failed_at, Some(0));
        gone.play(Sound::Select, RETRY_MS - 1);
        assert_eq!(gone.failed_at, Some(0));
    }

    #[test]
    fn a_player_that_dies_at_once_is_not_started_at_every_key() {
        let mut p = Player::new(Some("true".into()));
        p.play(Sound::Select, 5000);
        // It may be gone before the write: then the write failed.
        if let Some(pid) = p.child.as_mut().map(|(c, _)| {
            let _ = c.wait();
            c.id() as i32
        }) {
            assert!(!p.exited(pid + 1, 5100), "not the player");
            assert!(p.exited(pid, 5100));
        }
        assert!(p.failed_at.is_some());
        p.play(Sound::Select, 5200);
        assert!(p.child.is_none());
        // One that lived past SHORT_LIFE_MS just ended: the next sound
        // starts another.
        let (mut p, _) = stub("lived");
        p.play(Sound::Select, 0);
        let pid = p.child.as_ref().map(|(c, _)| c.id() as i32).unwrap();
        assert!(p.exited(pid, SHORT_LIFE_MS + 1));
        assert_eq!(p.failed_at, None);
        p.play(Sound::Select, SHORT_LIFE_MS + 2);
        assert!(p.child.is_some());
    }

    #[test]
    fn only_a_stream_after_the_amplifier_went_off_leads_with_silence() {
        let (mut p, _) = stub("lead");
        assert_eq!(p.lead_ms(0), LEAD_MS, "nothing played yet");
        p.play(Sound::Select, 0);
        assert_eq!(p.lead_ms(10), 0, "the stream is open");
        let close = p.deadline().unwrap();
        p.tick(close);
        // It ended when it closed, after the hold.
        assert_eq!(p.ended_at, Some(close));
        assert_eq!(p.lead_ms(close + AMP_OFF_MS - 1), 0);
        assert_eq!(p.lead_ms(close + AMP_OFF_MS), LEAD_MS);
        // Closed by the dark mid-sound: it ends once the sound has played.
        p.play(Sound::Notice, close + 100);
        let busy = p.busy_until;
        p.close(close + 110);
        assert_eq!(p.ended_at, Some(busy));
    }

    #[test]
    fn a_warm_up_opens_the_stream_and_a_sound_follows_its_silence() {
        let (mut p, out) = stub("warm");
        p.warm(0);
        assert_eq!(p.deadline(), Some(LEAD_MS + HOLD_MS));
        // A key during the silence plays after it, with no second lead.
        p.play(Sound::Select, 40);
        assert_eq!(p.busy_until, LEAD_MS + ms(Sound::Select));
        // Another warm-up while open writes nothing and keeps the hold.
        let held = p.deadline().unwrap();
        p.warm(50);
        assert_eq!(p.deadline(), Some(held));
        assert_eq!(played(&mut p, &out).len(), LEAD + bytes(Sound::Select));
        // Soon after a stream, the amplifier is still on: just enough
        // silence to start the stream.
        let (mut p, _) = stub("warm-again");
        p.ended_at = Some(1000);
        p.warm(2000);
        assert_eq!(p.busy_until, 2000 + START_MS);
    }

    #[test]
    fn after_a_dead_player_the_next_stream_counts_as_cold() {
        let (mut p, _) = stub("dead");
        p.ended_at = Some(0);
        p.play(Sound::Select, 1000);
        assert_eq!(p.busy_until, 1000 + ms(Sound::Select), "warm: no lead");
        let pid = p.child.as_ref().map(|(c, _)| c.id() as i32).unwrap();
        assert!(p.exited(pid, 1000 + SHORT_LIFE_MS));
        assert_eq!(p.ended_at, None);
        assert_eq!(p.lead_ms(1000 + SHORT_LIFE_MS + 1), LEAD_MS);
    }
}
