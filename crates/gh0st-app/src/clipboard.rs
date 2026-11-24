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
    /// What this app copied last, for when the session isn't there.
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

    /// False when the session refused: another app is in front, or it is
    /// too much.
    pub fn copy(&mut self, data: &[u8]) -> bool {
        let Some(c) = self.conn() else {
            self.local = Some(data.to_vec());
            return true;
        };
        let m = Message::call(NAME, PATH, IFACE, "Set")
            .arg(Value::str(TEXT))
            .arg(Value::Bytes(data.to_vec()));
        match c.call(m, CALL_MS) {
            Ok(_) => {
                self.local = Some(data.to_vec());
                true
            }
            Err(Error::Remote { .. }) => false,
            Err(_) => {
                // The bus went away: this app's own, and try again later.
                self.conn = None;
                self.tried = false;
                self.local = Some(data.to_vec());
                true
            }
        }
    }

    /// The text copied last, by this app or another.
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
