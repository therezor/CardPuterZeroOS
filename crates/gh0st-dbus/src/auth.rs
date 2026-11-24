//! SASL, EXTERNAL only: the kernel already told each side who the other is.

use crate::{Error, Result};

const MAX_LINE: usize = 512;
const MAX_LINES: usize = 16;

/// The client's whole side, sent at once as sd-bus does: the NUL, AUTH,
/// optionally NEGOTIATE_UNIX_FD, then BEGIN.
pub fn client_hello(uid: u32, fds: bool) -> Vec<u8> {
    let hex: String = uid
        .to_string()
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut out = format!("\0AUTH EXTERNAL {hex}\r\n");
    if fds {
        out.push_str("NEGOTIATE_UNIX_FD\r\n");
    }
    out.push_str("BEGIN\r\n");
    out.into_bytes()
}

/// Reads the server's answers to [`client_hello`] from lines.
#[derive(Default)]
pub struct Client {
    line: Vec<u8>,
    ok: bool,
    want_fds: bool,
    /// The server agreed to pass fds.
    pub fds: bool,
}

impl Client {
    pub fn new(want_fds: bool) -> Client {
        Client {
            want_fds,
            ..Client::default()
        }
    }

    /// Feeds one byte; true once every answer is in. Reads a byte at a
    /// time so it never eats the first message.
    pub fn feed(&mut self, b: u8) -> Result<bool> {
        self.line.push(b);
        if self.line.len() > MAX_LINE {
            return Err(Error::Invalid("auth line too long"));
        }
        if !self.line.ends_with(b"\r\n") {
            return Ok(false);
        }
        let line = String::from_utf8_lossy(&self.line[..self.line.len() - 2]).into_owned();
        self.line.clear();
        if !self.ok {
            if line.starts_with("OK ") {
                self.ok = true;
                return Ok(!self.want_fds);
            }
            return Err(Error::Invalid("the bus refused EXTERNAL"));
        }
        self.fds = line == "AGREE_UNIX_FD";
        Ok(true)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum St {
    Nul,
    Auth,
    Data,
    Ok,
    Done,
}

/// The server's side, fed whatever bytes arrive.
pub struct Server {
    uid: u32,
    guid: String,
    can_fds: bool,
    st: St,
    line: Vec<u8>,
    lines: usize,
    /// The client asked for fds and we agreed.
    pub fds: bool,
}

impl Server {
    pub fn new(peer_uid: u32, guid: &str, can_fds: bool) -> Server {
        Server {
            uid: peer_uid,
            guid: guid.into(),
            can_fds,
            st: St::Nul,
            line: Vec::new(),
            lines: 0,
            fds: false,
        }
    }

    pub fn done(&self) -> bool {
        self.st == St::Done
    }

    /// Takes bytes; answers go to `out`. Returns how many bytes it used
    /// once BEGIN arrives: the rest are messages.
    pub fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<Option<usize>> {
        for (i, &b) in input.iter().enumerate() {
            if self.st == St::Nul {
                if b != 0 {
                    return Err(Error::Invalid("no credentials byte"));
                }
                self.st = St::Auth;
                continue;
            }
            self.line.push(b);
            if self.line.len() > MAX_LINE {
                return Err(Error::Invalid("auth line too long"));
            }
            if !self.line.ends_with(b"\r\n") {
                continue;
            }
            self.lines += 1;
            if self.lines > MAX_LINES {
                return Err(Error::Invalid("too many auth lines"));
            }
            let line = String::from_utf8_lossy(&self.line[..self.line.len() - 2]).into_owned();
            self.line.clear();
            self.line_in(&line, out)?;
            if self.st == St::Done {
                return Ok(Some(i + 1));
            }
        }
        Ok(None)
    }

    fn line_in(&mut self, line: &str, out: &mut Vec<u8>) -> Result<()> {
        let (cmd, arg) = line.split_once(' ').unwrap_or((line, ""));
        match (self.st, cmd) {
            (St::Auth, "AUTH") => {
                let (mech, resp) = arg.split_once(' ').unwrap_or((arg, ""));
                if mech != "EXTERNAL" {
                    out.extend_from_slice(b"REJECTED EXTERNAL\r\n");
                } else if resp.is_empty() {
                    self.st = St::Data;
                    out.extend_from_slice(b"DATA\r\n");
                } else {
                    self.check(resp, out);
                }
            }
            (St::Data, "DATA") => self.check(arg, out),
            (St::Auth | St::Data, "CANCEL" | "ERROR") => {
                self.st = St::Auth;
                out.extend_from_slice(b"REJECTED EXTERNAL\r\n");
            }
            (St::Ok, "NEGOTIATE_UNIX_FD") => {
                if self.can_fds {
                    self.fds = true;
                    out.extend_from_slice(b"AGREE_UNIX_FD\r\n");
                } else {
                    out.extend_from_slice(b"ERROR fd passing not supported\r\n");
                }
            }
            (St::Ok, "BEGIN") => self.st = St::Done,
            (St::Ok, "CANCEL" | "ERROR") => {
                self.st = St::Auth;
                out.extend_from_slice(b"REJECTED EXTERNAL\r\n");
            }
            _ => out.extend_from_slice(b"ERROR unknown command\r\n"),
        }
        Ok(())
    }

    /// An empty identity means "whoever the kernel says"; otherwise it must
    /// be the peer's UID.
    fn check(&mut self, hex: &str, out: &mut Vec<u8>) {
        let who = unhex(hex);
        let ok = match who.as_deref() {
            Some("") => true,
            Some(s) => s.parse::<u32>().ok() == Some(self.uid),
            None => false,
        };
        if ok {
            self.st = St::Ok;
            out.extend_from_slice(format!("OK {}\r\n", self.guid).as_bytes());
        } else {
            self.st = St::Auth;
            out.extend_from_slice(b"REJECTED EXTERNAL\r\n");
        }
    }
}

fn unhex(s: &str) -> Option<String> {
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(server: &mut Server, input: &[u8]) -> (String, Option<usize>) {
        let mut out = Vec::new();
        let used = server.feed(input, &mut out).unwrap();
        (String::from_utf8(out).unwrap(), used)
    }

    #[test]
    fn a_libdbus_client_gets_in() {
        let mut s = Server::new(1000, "0123", true);
        let mut input = client_hello(1000, true);
        input.extend_from_slice(b"lmessage");
        let (out, used) = run(&mut s, &input);
        assert_eq!(out, "OK 0123\r\nAGREE_UNIX_FD\r\n");
        assert_eq!(&input[used.unwrap()..], b"lmessage");
        assert!(s.fds && s.done());
        let mut c = Client::new(true);
        let done: Vec<bool> = out.bytes().map(|b| c.feed(b).unwrap()).collect();
        assert!(*done.last().unwrap() && c.fds);
    }

    #[test]
    fn an_sd_bus_client_gets_in() {
        let mut s = Server::new(1000, "g", false);
        let (out, used) = run(
            &mut s,
            b"\0AUTH EXTERNAL\r\nDATA\r\nNEGOTIATE_UNIX_FD\r\nBEGIN\r\n",
        );
        assert_eq!(out, "DATA\r\nOK g\r\nERROR fd passing not supported\r\n");
        assert!(used.is_some() && !s.fds);
    }

    #[test]
    fn another_uid_or_mechanism_is_rejected() {
        let mut s = Server::new(1000, "g", true);
        let (out, used) = run(&mut s, &client_hello(0, false));
        assert!(out.starts_with("REJECTED"));
        assert!(used.is_none() && !s.done());
        let mut s = Server::new(1000, "g", true);
        let (out, _) = run(&mut s, b"\0AUTH DBUS_COOKIE_SHA1 31\r\n");
        assert_eq!(out, "REJECTED EXTERNAL\r\n");
        assert!(
            Server::new(1, "g", true)
                .feed(b"x", &mut Vec::new())
                .is_err()
        );
    }

    #[test]
    fn floods_stop() {
        let mut s = Server::new(1000, "g", true);
        let long = [b'A'; 600];
        let mut input = vec![0];
        input.extend_from_slice(&long);
        assert!(s.feed(&input, &mut Vec::new()).is_err());
        let mut s = Server::new(1000, "g", true);
        let mut input = vec![0];
        for _ in 0..20 {
            input.extend_from_slice(b"FOO\r\n");
        }
        assert!(s.feed(&input, &mut Vec::new()).is_err());
    }
}
