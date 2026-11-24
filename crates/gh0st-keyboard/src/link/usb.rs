//! The USB keyboard. Root builds the gadget (`gh0st-helper usb-hid hold`)
//! and holds it while this side keeps the helper's stdin open; the helper
//! prints each function's device node and the controller's state. Reports
//! go to the nodes the helper made ours.
//!
//! A node takes one report until the host collects it, so each keeps a
//! queue, drained on a short timer only while it isn't empty: a press and
//! its release sent back to back both arrive.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};

use super::{Cable, Event, UsbState};
use crate::hid::report::Report;

const HELPER: &str = "/usr/libexec/gh0st-helper";
/// The USB hub is on the bus only while the side switch is on Host.
pub const HUB: &str = "/sys/bus/usb/devices/1-1";
/// Reports waiting for one node: more means the host isn't reading.
const QUEUE_MAX: usize = 64;
/// How often queued reports are tried again, while any wait.
pub const TICK_MS: u64 = 4;

/// One function's device node.
pub trait Node {
    fn fd(&self) -> RawFd;
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize>;
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
}

impl Node for std::fs::File {
    fn fd(&self) -> RawFd {
        self.as_raw_fd()
    }
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        Write::write(self, data)
    }
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        Read::read(self, buf)
    }
}

struct Port {
    node: Box<dyn Node>,
    queue: VecDeque<([u8; 8], usize)>,
}

impl Port {
    /// Writes what is queued until the node is full.
    fn flush(&mut self) {
        while let Some((buf, n)) = self.queue.front() {
            match self.node.write(&buf[..*n]) {
                Ok(_) => {
                    self.queue.pop_front();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
                // The host went: what waits is lost with it.
                Err(_) => {
                    self.queue.clear();
                    return;
                }
            }
        }
    }
}

const KEYBOARD: usize = 0;
const MOUSE: usize = 1;
const CONSUMER: usize = 2;

pub struct Usb {
    argv: Vec<String>,
    hub: PathBuf,
    state: UsbState,
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    pid: Option<i32>,
    /// Spawned, and not yet handed to the SDK to reap.
    fresh: Option<i32>,
    line: Vec<u8>,
    /// The controller's state word, as the kernel says it.
    udc: String,
    ports: [Option<Port>; 3],
    #[cfg(target_os = "linux")]
    uevents: Option<gh0st_core::status::Watch>,
    /// Why it failed, for the user.
    pub why: Option<String>,
}

impl Usb {
    /// The USB keyboard on the device.
    pub fn new() -> Usb {
        let argv = ["pkexec", HELPER, "usb-hid", "hold"];
        Usb::with(
            argv.iter().map(|s| s.to_string()).collect(),
            PathBuf::from(HUB),
        )
    }

    /// With another helper command and hub path: for tests.
    pub fn with(argv: Vec<String>, hub: PathBuf) -> Usb {
        Usb {
            argv,
            hub,
            state: UsbState::Off,
            stdin: None,
            stdout: None,
            pid: None,
            fresh: None,
            line: Vec::new(),
            udc: String::new(),
            ports: [None, None, None],
            #[cfg(target_os = "linux")]
            uevents: None,
            why: None,
        }
    }

    /// Nodes already open and a host on them: for tests of the queues.
    pub fn connected(nodes: [Box<dyn Node>; 3]) -> Usb {
        let mut u = Usb::with(Vec::new(), PathBuf::new());
        u.state = UsbState::Connected;
        let [k, m, c] = nodes;
        u.ports = [k, m, c].map(|node| {
            Some(Port {
                node,
                queue: VecDeque::new(),
            })
        });
        u
    }

    fn start(&mut self) {
        let Some((cmd, args)) = self.argv.split_first() else {
            return;
        };
        let child = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                self.state = UsbState::Failed;
                self.why = Some(e.to_string());
                return;
            }
        };
        let (stdin, stdout) = (child.stdin.take(), child.stdout.take());
        if let Some(out) = &stdout {
            let _ = gh0st_core::rt::set_nonblock_cloexec(out.as_raw_fd());
        }
        self.stdin = stdin;
        self.stdout = stdout;
        let pid = child.id() as i32;
        self.pid = Some(pid);
        self.fresh = Some(pid);
        self.state = UsbState::Starting;
        self.why = None;
        #[cfg(target_os = "linux")]
        {
            self.uevents = gh0st_core::status::Watch::uevents().ok();
        }
    }

    /// Lets go: stdin closes, so the helper takes the gadget down and the
    /// host sees the keyboard unplugged.
    fn close(&mut self) {
        self.stdin = None;
        self.stdout = None;
        self.ports = [None, None, None];
        self.line.clear();
        self.udc.clear();
        #[cfg(target_os = "linux")]
        {
            self.uevents = None;
        }
    }

    /// The state from the controller's word and where the switch is.
    fn settle(&mut self) {
        if self.stdin.is_none() {
            return;
        }
        self.state = match self.udc.as_str() {
            "" => UsbState::Starting,
            "configured" => UsbState::Connected,
            "suspended" => UsbState::Asleep,
            "not attached" if self.hub.exists() => UsbState::HostMode,
            _ => UsbState::NoCable,
        };
    }

    fn line(&mut self, line: &str) {
        let mut w = line.splitn(3, ' ');
        match (w.next(), w.next(), w.next()) {
            (Some("node"), Some(name), Some(path)) => {
                let i = match name {
                    "keyboard" => KEYBOARD,
                    "mouse" => MOUSE,
                    "consumer" => CONSUMER,
                    _ => return,
                };
                match open(Path::new(path)) {
                    Ok(f) => {
                        self.ports[i] = Some(Port {
                            node: Box::new(f),
                            queue: VecDeque::new(),
                        });
                    }
                    Err(e) => self.why = Some(format!("{path}: {e}")),
                }
            }
            (Some("state"), Some(first), rest) => {
                self.udc = match rest {
                    Some(r) => format!("{first} {r}"),
                    None => first.to_string(),
                };
            }
            _ => {}
        }
    }

    fn read_helper(&mut self) {
        let Some(out) = &mut self.stdout else { return };
        let mut buf = [0u8; 512];
        loop {
            match out.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.line.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        while let Some(i) = self.line.iter().position(|&b| b == b'\n') {
            let l: Vec<u8> = self.line.drain(..=i).collect();
            let s = String::from_utf8_lossy(&l[..l.len() - 1]).into_owned();
            self.line(&s);
        }
    }

    /// The helper ended. A clean end after we let go is Off; anything else
    /// is a failure.
    pub fn ended(&mut self, pid: i32, code: Option<i32>, out: &mut Vec<Event>) {
        if self.pid != Some(pid) {
            return;
        }
        self.pid = None;
        let ours = self.stdin.is_none();
        self.close();
        self.state = if ours || code == Some(0) {
            UsbState::Off
        } else {
            UsbState::Failed
        };
        if self.state == UsbState::Failed && self.why.is_none() {
            self.why = Some(match code {
                // pkexec: not allowed, or dismissed.
                Some(126 | 127) => "not allowed".into(),
                Some(c) => format!("the helper stopped ({c})"),
                None => "the helper was killed".into(),
            });
        }
        out.push(Event::Usb(self.state));
    }
}

impl Default for Usb {
    fn default() -> Self {
        Usb::new()
    }
}

fn open(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
}

impl Cable for Usb {
    fn fds(&self) -> Vec<RawFd> {
        let mut v: Vec<RawFd> = self.stdout.iter().map(|o| o.as_raw_fd()).collect();
        // The keyboard's node is readable when the host sets its lights.
        if let Some(p) = &self.ports[KEYBOARD] {
            v.push(p.node.fd());
        }
        #[cfg(target_os = "linux")]
        if let Some(w) = &self.uevents {
            v.push(w.fd());
        }
        v
    }

    fn ready(&mut self, fd: RawFd, out: &mut Vec<Event>) {
        let before = self.state;
        if self.stdout.as_ref().map(|o| o.as_raw_fd()) == Some(fd) {
            self.read_helper();
        } else if let Some(p) = self.ports[KEYBOARD].as_mut().filter(|p| p.node.fd() == fd) {
            let mut b = [0u8; 8];
            if let Ok(n) = p.node.read(&mut b)
                && n >= 1
            {
                out.push(Event::Leds(b[0]));
            }
        }
        #[cfg(target_os = "linux")]
        if let Some(w) = self.uevents.as_ref().filter(|w| w.fd() == fd) {
            // The hub came or went: the side switch moved.
            w.drain();
        }
        self.settle();
        if self.state != before {
            out.push(Event::Usb(self.state));
        }
    }

    fn state(&self) -> UsbState {
        self.state
    }

    fn enable(&mut self, on: bool) {
        if on && self.pid.is_none() {
            self.start();
        } else if !on {
            self.close();
            self.state = UsbState::Off;
        }
    }

    fn send(&mut self, r: &Report) -> bool {
        if self.state != UsbState::Connected {
            return false;
        }
        let i = match r {
            Report::Keyboard(_) => KEYBOARD,
            Report::Mouse(_) => MOUSE,
            Report::Consumer(_) => CONSUMER,
        };
        let Some(p) = &mut self.ports[i] else {
            return false;
        };
        if p.queue.len() >= QUEUE_MAX {
            return false;
        }
        let mut buf = [0u8; 8];
        let n = r.bytes(&mut buf);
        p.queue.push_back((buf, n));
        p.flush();
        true
    }

    fn tick(&mut self) {
        for p in self.ports.iter_mut().flatten() {
            p.flush();
        }
    }

    fn waiting(&self) -> bool {
        self.ports.iter().flatten().any(|p| !p.queue.is_empty())
    }

    fn spawned(&mut self) -> Option<i32> {
        self.fresh.take()
    }

    fn exited(&mut self, pid: i32, code: Option<i32>, out: &mut Vec<Event>) {
        self.ended(pid, code, out);
    }

    fn stop(&mut self) {
        self.close();
        self.state = UsbState::Off;
    }
}
