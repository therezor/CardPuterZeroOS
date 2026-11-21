//! Keys from a script on stdin, for `--sim DIR` runs on a host (the shell and
//! every SDK app). One step per line:
//!
//! ```text
//! s                 tap a key: a letter, digit or a name (enter, esc, tab...)
//! fn+z              tap with a modifier held (fn, shift, ctrl, alt)
//! type notes        tap each character
//! down esc          press and hold
//! up esc            release
//! wait 3500         let time pass, in milliseconds
//! # comment
//! ```

use crate::keys::{self, code};
use std::collections::VecDeque;
use std::os::fd::RawFd;

const TAP_GAP_MS: u64 = 30;

pub struct Script {
    fd: RawFd,
    buf: Vec<u8>,
    queue: VecDeque<(u64, u16, i32)>,
    /// When the last queued event happens; later lines follow it.
    cursor: u64,
    eof: bool,
}

impl Script {
    pub fn stdin() -> Script {
        let _ = crate::rt::set_nonblock_cloexec(0);
        Script {
            fd: 0,
            buf: Vec::new(),
            queue: VecDeque::new(),
            cursor: 0,
            eof: false,
        }
    }

    /// A script given as text, already ended: for tests.
    pub fn from_text(text: &str) -> Script {
        let mut s = Script {
            fd: -1,
            buf: Vec::new(),
            queue: VecDeque::new(),
            cursor: 0,
            eof: true,
        };
        for l in text.lines() {
            s.parse(l.trim(), 0);
        }
        s
    }

    /// Every queued event, in order, without times.
    pub fn drain_all(&mut self) -> Vec<(u16, i32)> {
        self.queue.drain(..).map(|(_, c, v)| (c, v)).collect()
    }

    /// Stdin until it ends, then nothing (poll ignores a negative fd).
    pub fn fd(&self) -> RawFd {
        if self.eof { -1 } else { self.fd }
    }

    /// Done once the script has ended, every key is sent and a trailing
    /// `wait` has run out, so a session can end on a settled frame.
    pub fn finished(&self, now: u64) -> bool {
        self.eof && self.queue.is_empty() && now >= self.cursor
    }

    pub fn next_at(&self) -> Option<u64> {
        match self.queue.front() {
            Some(e) => Some(e.0),
            None if self.eof => Some(self.cursor),
            None => None,
        }
    }

    pub fn read(&mut self, now: u64) {
        let mut chunk = [0u8; 4096];
        loop {
            // SAFETY: reading into a local buffer of its stated size.
            let n = unsafe {
                libc::read(
                    self.fd,
                    chunk.as_mut_ptr() as *mut libc::c_void,
                    chunk.len(),
                )
            };
            if n == 0 {
                self.eof = true;
                break;
            }
            if n < 0 {
                break;
            }
            self.buf.extend_from_slice(&chunk[..n as usize]);
        }
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            self.parse(String::from_utf8_lossy(&line).trim(), now);
        }
        if self.eof && !self.buf.is_empty() {
            let line = String::from_utf8_lossy(&std::mem::take(&mut self.buf))
                .trim()
                .to_string();
            self.parse(&line, now);
        }
    }

    fn at(&mut self, now: u64) -> u64 {
        self.cursor = self.cursor.max(now) + TAP_GAP_MS;
        self.cursor
    }

    fn push(&mut self, now: u64, code: u16, value: i32) {
        let t = self.at(now);
        self.queue.push_back((t, code, value));
    }

    fn tap(&mut self, now: u64, code: u16) {
        self.push(now, code, 1);
        self.push(now, code, 0);
    }

    fn parse(&mut self, line: &str, now: u64) {
        if line.is_empty() || line.starts_with('#') {
            return;
        }
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        match verb {
            "wait" => self.cursor = self.cursor.max(now) + rest.trim().parse::<u64>().unwrap_or(0),
            "type" => {
                for c in rest.chars() {
                    if let Some(k) = keys::code_for_char(c.to_ascii_lowercase()) {
                        let shift = c.is_ascii_uppercase();
                        if shift {
                            self.push(now, code::LEFTSHIFT, 1);
                        }
                        self.tap(now, k);
                        if shift {
                            self.push(now, code::LEFTSHIFT, 0);
                        }
                    }
                }
            }
            "down" | "up" => match keys::code_for_name(rest.trim()) {
                Some(k) => self.push(now, k, (verb == "down") as i32),
                None => eprintln!("sim: unknown key {rest:?}"),
            },
            _ => {
                let parts: Vec<&str> = line.split('+').collect();
                let Some((key, mods)) = parts.split_last() else {
                    return;
                };
                let codes: Option<Vec<u16>> = mods
                    .iter()
                    .chain(std::iter::once(key))
                    .map(|n| keys::code_for_name(n))
                    .collect();
                let Some(codes) = codes else {
                    eprintln!("sim: unknown key {line:?}");
                    return;
                };
                let (k, mods) = codes.split_last().unwrap();
                for &m in mods {
                    self.push(now, m, 1);
                }
                self.tap(now, *k);
                for &m in mods.iter().rev() {
                    self.push(now, m, 0);
                }
            }
        }
    }

    /// Events whose time has come.
    pub fn due(&mut self, now: u64, out: &mut Vec<(u16, i32)>) {
        while self.queue.front().is_some_and(|e| e.0 <= now) {
            let (_, c, v) = self.queue.pop_front().unwrap();
            out.push((c, v));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(text: &str) -> Vec<(u16, i32)> {
        let mut s = Script {
            fd: -1,
            buf: Vec::new(),
            queue: VecDeque::new(),
            cursor: 0,
            eof: true,
        };
        for l in text.lines() {
            s.parse(l.trim(), 0);
        }
        s.queue.into_iter().map(|(_, c, v)| (c, v)).collect()
    }

    #[test]
    fn taps_chords_and_holds() {
        assert_eq!(script("s"), [(31, 1), (31, 0)]);
        assert_eq!(
            script("fn+z"),
            [(code::FN, 1), (44, 1), (44, 0), (code::FN, 0)]
        );
        assert_eq!(script("down esc\nup esc"), [(code::ESC, 1), (code::ESC, 0)]);
        assert_eq!(script("type Ab").len(), 6);
    }

    #[test]
    fn wait_pushes_later_steps_out() {
        let mut s = Script {
            fd: -1,
            buf: Vec::new(),
            queue: VecDeque::new(),
            cursor: 0,
            eof: true,
        };
        s.parse("down esc", 1000);
        s.parse("wait 3500", 1000);
        s.parse("up esc", 1000);
        let times: Vec<u64> = s.queue.iter().map(|e| e.0).collect();
        assert_eq!(times[1] - times[0], 3500 + TAP_GAP_MS);
    }
}
