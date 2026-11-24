//! A whole message: the fixed header, the header fields, the body.

use std::collections::VecDeque;
use std::os::fd::OwnedFd;

use crate::sig::{self, Ty};
use crate::value::Value;
use crate::wire::{Reader, Writer};
use crate::{Error, MAX_MESSAGE, Result, names};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Call = 1,
    Return = 2,
    Error = 3,
    Signal = 4,
}

pub const NO_REPLY: u8 = 0x1;
pub const NO_AUTO_START: u8 = 0x2;
pub const ALLOW_INTERACTIVE: u8 = 0x4;

const PATH: u8 = 1;
const INTERFACE: u8 = 2;
const MEMBER: u8 = 3;
const ERROR_NAME: u8 = 4;
const REPLY_SERIAL: u8 = 5;
const DESTINATION: u8 = 6;
const SENDER: u8 = 7;
const SIGNATURE: u8 = 8;
const UNIX_FDS: u8 = 9;

#[derive(Debug)]
pub struct Message {
    pub kind: Kind,
    pub flags: u8,
    /// Set when sent or received.
    pub serial: u32,
    pub path: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub error_name: Option<String>,
    pub reply_serial: Option<u32>,
    pub destination: Option<String>,
    pub sender: Option<String>,
    pub body: Vec<Value>,
    pub fds: Vec<OwnedFd>,
}

impl Message {
    fn new(kind: Kind) -> Message {
        Message {
            kind,
            flags: 0,
            serial: 0,
            path: None,
            interface: None,
            member: None,
            error_name: None,
            reply_serial: None,
            destination: None,
            sender: None,
            body: Vec::new(),
            fds: Vec::new(),
        }
    }

    pub fn call(dest: &str, path: &str, iface: &str, member: &str) -> Message {
        Message {
            destination: Some(dest.into()),
            path: Some(path.into()),
            interface: Some(iface.into()),
            member: Some(member.into()),
            ..Message::new(Kind::Call)
        }
    }

    pub fn signal(path: &str, iface: &str, member: &str) -> Message {
        Message {
            path: Some(path.into()),
            interface: Some(iface.into()),
            member: Some(member.into()),
            ..Message::new(Kind::Signal)
        }
    }

    pub fn reply(call: &Message) -> Message {
        Message {
            reply_serial: Some(call.serial),
            destination: call.sender.clone(),
            ..Message::new(Kind::Return)
        }
    }

    pub fn error(call: &Message, name: &str, text: &str) -> Message {
        Message {
            reply_serial: Some(call.serial),
            destination: call.sender.clone(),
            error_name: Some(name.into()),
            body: vec![Value::str(text)],
            ..Message::new(Kind::Error)
        }
    }

    pub fn arg(mut self, v: Value) -> Message {
        self.body.push(v);
        self
    }

    pub fn flags(mut self, flags: u8) -> Message {
        self.flags |= flags;
        self
    }

    /// True for a call whose sender wants no reply.
    pub fn no_reply(&self) -> bool {
        self.flags & NO_REPLY != 0
    }

    pub fn signature(&self) -> String {
        self.body.iter().map(|v| v.ty().sig()).collect()
    }

    /// Interface and member, for matching a call.
    pub fn is(&self, iface: &str, member: &str) -> bool {
        self.interface.as_deref() == Some(iface) && self.member.as_deref() == Some(member)
    }

    /// The text of an error reply.
    pub fn error_text(&self) -> String {
        self.body
            .first()
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }

    /// Zeroes the body: for messages that carried a secret.
    pub fn wipe(&mut self) {
        self.body.iter_mut().for_each(Value::wipe);
    }

    pub fn encode(&self, serial: u32) -> Result<Vec<u8>> {
        if serial == 0 {
            return Err(Error::Invalid("serial is zero"));
        }
        self.check()?;
        let mut body = Writer::new();
        for v in &self.body {
            body.value(v)?;
        }
        let sig = self.signature();
        if sig.len() > sig::MAX_SIG {
            return Err(Error::Invalid("signature too long"));
        }
        let mut w = Writer::new();
        w.u8(b'l');
        w.u8(self.kind as u8);
        w.u8(self.flags);
        w.u8(1);
        w.u32(body.buf.len() as u32);
        w.u32(serial);
        let mut fields: Vec<(u8, Value)> = Vec::new();
        let s = |o: &Option<String>| o.clone().map(Value::Str);
        if let Some(p) = &self.path {
            fields.push((PATH, Value::Path(p.clone())));
        }
        for (code, v) in [
            (INTERFACE, s(&self.interface)),
            (MEMBER, s(&self.member)),
            (ERROR_NAME, s(&self.error_name)),
            (REPLY_SERIAL, self.reply_serial.map(Value::U32)),
            (DESTINATION, s(&self.destination)),
            (SENDER, s(&self.sender)),
        ] {
            if let Some(v) = v {
                fields.push((code, v));
            }
        }
        if !sig.is_empty() {
            fields.push((SIGNATURE, Value::Sig(sig)));
        }
        if !self.fds.is_empty() {
            fields.push((UNIX_FDS, Value::U32(self.fds.len() as u32)));
        }
        let header = Value::Array(
            Ty::Struct(vec![Ty::Byte, Ty::Variant]),
            fields
                .into_iter()
                .map(|(c, v)| Value::Struct(vec![Value::Byte(c), Value::variant(v)]))
                .collect(),
        );
        w.value(&header)?;
        w.pad(8);
        w.buf.extend_from_slice(&body.buf);
        crate::wipe(&mut body.buf);
        if w.buf.len() > MAX_MESSAGE {
            return Err(Error::Invalid("message too long"));
        }
        Ok(w.buf)
    }

    /// The fields each kind must have, and names that follow the rules.
    fn check(&self) -> Result<()> {
        let need = |o: &Option<String>, what| o.as_ref().map(|_| ()).ok_or(Error::Invalid(what));
        match self.kind {
            Kind::Call => {
                need(&self.path, "call without a path")?;
                need(&self.member, "call without a member")?;
            }
            Kind::Signal => {
                need(&self.path, "signal without a path")?;
                need(&self.interface, "signal without an interface")?;
                need(&self.member, "signal without a member")?;
            }
            Kind::Error => {
                need(&self.error_name, "error without a name")?;
                self.reply_serial
                    .ok_or(Error::Invalid("error without a reply serial"))?;
            }
            Kind::Return => {
                self.reply_serial
                    .ok_or(Error::Invalid("return without a reply serial"))?;
            }
        }
        let ok = |o: &Option<String>, f: fn(&str) -> bool| o.as_deref().is_none_or(f);
        if !ok(&self.path, names::path)
            || !ok(&self.interface, names::interface)
            || !ok(&self.member, names::member)
            || !ok(&self.error_name, names::error)
            || !ok(&self.destination, names::bus)
            || !ok(&self.sender, names::bus)
        {
            return Err(Error::Invalid("bad name in the header"));
        }
        if self.reply_serial == Some(0) {
            return Err(Error::Invalid("reply serial is zero"));
        }
        Ok(())
    }

    /// The whole message's length from its first 16 bytes, once they are
    /// in. Errors on a length past [`MAX_MESSAGE`].
    pub fn frame_len(head: &[u8]) -> Result<Option<usize>> {
        let Some(h) = head.get(..16) else {
            return Ok(None);
        };
        let big = match h[0] {
            b'l' => false,
            b'B' => true,
            _ => return Err(Error::Invalid("bad endian mark")),
        };
        if h[3] != 1 {
            return Err(Error::Invalid("bad protocol version"));
        }
        let word = |at: usize| {
            let mut a = [0u8; 4];
            a.copy_from_slice(&h[at..at + 4]);
            if big {
                u32::from_be_bytes(a)
            } else {
                u32::from_le_bytes(a)
            }
        } as usize;
        let body = word(4);
        let fields = word(12);
        if body > MAX_MESSAGE || fields > MAX_MESSAGE {
            return Err(Error::Invalid("message too long"));
        }
        let total = (16 + fields).div_ceil(8) * 8 + body;
        if total > MAX_MESSAGE {
            return Err(Error::Invalid("message too long"));
        }
        Ok(Some(total))
    }

    /// One message from exactly its bytes. It takes the fds the header
    /// counts from the front of `fds`.
    pub fn decode(bytes: &[u8], fds: &mut VecDeque<OwnedFd>) -> Result<Message> {
        let total = Message::frame_len(bytes)?.ok_or(Error::Invalid("message too short"))?;
        if total != bytes.len() {
            return Err(Error::Invalid("length mismatch"));
        }
        let big = bytes[0] == b'B';
        let mut r = Reader::new(bytes, big);
        r.pos = 1;
        let kind = match r.u8()? {
            1 => Kind::Call,
            2 => Kind::Return,
            3 => Kind::Error,
            4 => Kind::Signal,
            _ => return Err(Error::Invalid("bad message type")),
        };
        let flags = r.u8()?;
        r.u8()?;
        let body_len = r.u32()? as usize;
        let serial = r.u32()?;
        if serial == 0 {
            return Err(Error::Invalid("serial is zero"));
        }
        let mut m = Message::new(kind);
        m.flags = flags;
        m.serial = serial;
        let header = r.value(&Ty::Array(Box::new(Ty::Struct(vec![
            Ty::Byte,
            Ty::Variant,
        ]))))?;
        let mut sig = String::new();
        let mut nfds = 0u32;
        let mut seen = 0u32;
        for f in header.as_array().unwrap_or(&[]) {
            let (Some(Value::Byte(code)), Some(v)) = (
                f.as_struct().and_then(|s| s.first()),
                f.as_struct().and_then(|s| s.get(1)).map(Value::inner),
            ) else {
                return Err(Error::Invalid("bad header field"));
            };
            if (1..=9).contains(code) {
                if seen & (1 << code) != 0 {
                    return Err(Error::Invalid("header field twice"));
                }
                seen |= 1 << code;
            }
            let text = |want: fn(&Value) -> bool| -> Result<String> {
                if want(v) {
                    Ok(v.as_str().unwrap_or_default().to_owned())
                } else {
                    Err(Error::Invalid("header field of the wrong type"))
                }
            };
            let is_str = |v: &Value| matches!(v, Value::Str(_));
            match *code {
                PATH => m.path = Some(text(|v| matches!(v, Value::Path(_)))?),
                INTERFACE => m.interface = Some(text(is_str)?),
                MEMBER => m.member = Some(text(is_str)?),
                ERROR_NAME => m.error_name = Some(text(is_str)?),
                DESTINATION => m.destination = Some(text(is_str)?),
                SENDER => m.sender = Some(text(is_str)?),
                SIGNATURE => sig = text(|v| matches!(v, Value::Sig(_)))?,
                REPLY_SERIAL => {
                    m.reply_serial = Some(v.as_u32().ok_or(Error::Invalid("bad reply serial"))?)
                }
                UNIX_FDS => nfds = v.as_u32().ok_or(Error::Invalid("bad fd count"))?,
                _ => {}
            }
        }
        m.check()?;
        r.align(8)?;
        if r.pos + body_len != bytes.len() {
            return Err(Error::Invalid("body length mismatch"));
        }
        if nfds as usize > fds.len() {
            return Err(Error::Invalid("fds missing"));
        }
        m.fds = fds.drain(..nfds as usize).collect();
        // Offsets stay counted from the message start: the body is 8-aligned.
        r.fds = nfds;
        for ty in sig::parse(&sig)? {
            m.body.push(r.value(&ty)?);
        }
        if r.pos != bytes.len() {
            return Err(Error::Invalid("body longer than its signature"));
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Hello with its fields in libdbus's order (path, destination,
    /// interface, member): 110 bytes of fields.
    const HELLO: &[u8] = b"l\x01\x00\x01\x00\x00\x00\x00\x01\x00\x00\x00n\x00\x00\x00\
\x01\x01o\x00\x15\x00\x00\x00/org/freedesktop/DBus\x00\x00\x00\
\x06\x01s\x00\x14\x00\x00\x00org.freedesktop.DBus\x00\x00\x00\x00\
\x02\x01s\x00\x14\x00\x00\x00org.freedesktop.DBus\x00\x00\x00\x00\
\x03\x01s\x00\x05\x00\x00\x00Hello\x00\x00\x00";

    #[test]
    fn reads_a_real_hello() {
        let m = Message::decode(HELLO, &mut VecDeque::new()).unwrap();
        assert_eq!(m.kind, Kind::Call);
        assert_eq!(m.serial, 1);
        assert_eq!(m.path.as_deref(), Some("/org/freedesktop/DBus"));
        assert!(m.is("org.freedesktop.DBus", "Hello"));
        assert_eq!(m.destination.as_deref(), Some("org.freedesktop.DBus"));
        assert!(m.body.is_empty());
    }

    #[test]
    fn writes_hello_like_libdbus() {
        let m = Message::call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "Hello",
        );
        let bytes = m.encode(1).unwrap();
        // Same fields, our order: compare by decoding both.
        let a = Message::decode(&bytes, &mut VecDeque::new()).unwrap();
        let b = Message::decode(HELLO, &mut VecDeque::new()).unwrap();
        assert_eq!(
            (a.path, a.interface, a.member, a.destination),
            (b.path, b.interface, b.member, b.destination)
        );
        assert_eq!(bytes.len(), HELLO.len());
    }

    #[test]
    fn a_body_round_trips() {
        let call = Message::call(
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
            "SearchItems",
        )
        .arg(Value::str_dict([("service", "git")]));
        let bytes = call.encode(9).unwrap();
        let m = Message::decode(&bytes, &mut VecDeque::new()).unwrap();
        assert_eq!(m.signature(), "a{ss}");
        assert_eq!(
            m.body[0].get("service").and_then(Value::as_str),
            Some("git")
        );
        assert_eq!(Message::frame_len(&bytes[..16]).unwrap(), Some(bytes.len()));
    }

    #[test]
    fn replies_and_errors_need_their_fields() {
        let mut call = Message::call(":1.1", "/", "a.b", "C");
        call.serial = 5;
        call.sender = Some(":1.7".into());
        let r = Message::reply(&call);
        assert_eq!(r.destination.as_deref(), Some(":1.7"));
        let bytes = Message::error(&call, "org.freedesktop.DBus.Error.Failed", "no")
            .encode(2)
            .unwrap();
        let e = Message::decode(&bytes, &mut VecDeque::new()).unwrap();
        assert_eq!(e.reply_serial, Some(5));
        assert_eq!(e.error_text(), "no");
        let mut bad = Message::reply(&call);
        bad.reply_serial = None;
        assert!(bad.encode(1).is_err());
        assert!(
            Message::call("x", "/", "a.b", "C").encode(1).is_err(),
            "bad destination"
        );
    }

    #[test]
    fn duplicate_and_mistyped_fields_are_refused() {
        let mut twice = HELLO.to_vec();
        // Turn the destination field (6) into a second path... as a string: mistyped.
        let at = twice
            .windows(4)
            .position(|w| w == b"\x06\x01s\x00")
            .unwrap();
        twice[at] = 1;
        assert!(Message::decode(&twice, &mut VecDeque::new()).is_err());
        let mut dup = HELLO.to_vec();
        let at = dup.windows(4).position(|w| w == b"\x02\x01s\x00").unwrap();
        dup[at] = 6;
        assert!(Message::decode(&dup, &mut VecDeque::new()).is_err());
    }

    #[test]
    fn lengths_past_the_limit_stop_early() {
        let mut h = HELLO[..16].to_vec();
        h[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Message::frame_len(&h).is_err());
        assert_eq!(Message::frame_len(&HELLO[..10]).unwrap(), None);
    }
}
