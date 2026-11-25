//! The clipboard every app shares, `org.gh0st.Clipboard` in `gh0st
//! session`. Calls are short and block: the session is local. Without it
//! (off the device, under `--sim`, in a test) the app keeps its own.

use gh0st_dbus::{Conn, Error, Message, Value};

const NAME: &str = "org.gh0st.Clipboard";
const PATH: &str = "/org/gh0st/Clipboard";
const IFACE: &str = "org.gh0st.Clipboard";
const TEXT: &str = "text/plain;charset=utf-8";
const CALL_MS: u64 = 2000;

#[derive(Default)]
pub(crate) struct Clip {
    conn: Option<Conn>,
    tried: bool,
    /// No session bus: the app's own clipboard only.
    pub offline: bool,
    /// What this app copied last while the session wasn't there. Never
    /// what the session took: the lock clears that, and a copy here
    /// would bring it back.
    pub local: Option<Vec<u8>>,
}

impl Clip {
    fn conn(&mut self) -> Option<&mut Conn> {
        if self.conn.is_none() && !self.tried && !self.offline {
            self.tried = true;
            self.conn = Conn::session().ok();
        }
        self.conn.as_mut()
    }

    /// Replaces this app's own copy. A password may be in the old one.
    pub fn keep(&mut self, data: Option<&[u8]>) {
        if let Some(old) = self.local.as_mut() {
            old.fill(0);
        }
        self.local = data.map(<[u8]>::to_vec);
    }

    /// False when the session refused: another app is in front, or it is
    /// too much.
    pub fn copy(&mut self, data: &[u8]) -> bool {
        let Some(c) = self.conn() else {
            self.keep(Some(data));
            return true;
        };
        let m = Message::call(NAME, PATH, IFACE, "Set")
            .arg(Value::str(TEXT))
            .arg(Value::Bytes(data.to_vec()));
        match c.call(m, CALL_MS) {
            Ok(_) => {
                self.keep(None);
                true
            }
            Err(Error::Remote { .. }) => false,
            Err(_) => {
                // The bus went away: this app's own, and try again later.
                self.conn = None;
                self.tried = false;
                self.keep(Some(data));
                true
            }
        }
    }

    /// The text copied last, by this app or another. None when the
    /// session has nothing (the lock cleared it) or refused: never this
    /// app's own copy then.
    pub fn paste(&mut self) -> Option<Vec<u8>> {
        let Some(c) = self.conn() else {
            return self.local.clone();
        };
        let m = Message::call(NAME, PATH, IFACE, "Get").arg(Value::str(TEXT));
        match c.call(m, CALL_MS) {
            Ok(r) => r.body.first().and_then(Value::as_bytes).map(<[u8]>::to_vec),
            Err(Error::Remote { .. }) => None,
            Err(_) => {
                self.conn = None;
                self.tried = false;
                self.local.clone()
            }
        }
    }
}

/// What Ctrl+V types into a field: the first line copied that has text,
/// at most `max` characters, cut before it is decoded so a big clipboard
/// costs no more than the line. Control characters end it; bytes that
/// aren't UTF-8 are left out, never typed as U+FFFD.
pub(crate) fn first_line(bytes: &[u8], max: usize) -> String {
    let line = bytes
        .split(|&b| b == b'\n' || b == b'\r')
        .find(|l| l.iter().any(|b| !b.is_ascii_whitespace()))
        .unwrap_or_default();
    // A character is 4 bytes at most.
    let line = &line[..line.len().min(max.saturating_mul(4))];
    let mut out = String::new();
    'chunks: for chunk in line.utf8_chunks() {
        for c in chunk.valid().chars() {
            if c.is_control() || out.chars().count() >= max {
                break 'chunks;
            }
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_gets_the_first_line_with_text() {
        assert_eq!(first_line(b"\n  \r\nhello world\nnext", 100), "hello world");
        assert_eq!(first_line(b"ab\xffcd\tef", 100), "abcd");
        assert_eq!(first_line("héllo".as_bytes(), 3), "hél");
        assert_eq!(first_line(b"", 10), "");
        // A big clipboard: only the line's bytes are looked at.
        let mut big = b"pass".to_vec();
        big.extend(std::iter::repeat_n(b'x', 1 << 20));
        assert_eq!(first_line(&big, 8), "passxxxx");
    }
}
