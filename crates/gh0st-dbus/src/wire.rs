//! Marshalling. We write little endian; we read either. Offsets count from
//! the message start, so alignment matches the spec.

use crate::sig::{self, MAX_ARRAYS, MAX_STRUCTS, Ty};
use crate::value::Value;
use crate::{Error, Result, names};

/// The spec's bound on one array's bytes.
const MAX_ARRAY: usize = 1 << 26;
/// Containers of any kind, variants included, as libdbus counts them.
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Default)]
struct Depth {
    arrays: usize,
    structs: usize,
    total: usize,
}

impl Depth {
    fn array(self) -> Result<Depth> {
        let d = Depth {
            arrays: self.arrays + 1,
            total: self.total + 1,
            ..self
        };
        d.check()
    }

    fn strukt(self) -> Result<Depth> {
        let d = Depth {
            structs: self.structs + 1,
            total: self.total + 1,
            ..self
        };
        d.check()
    }

    fn variant(self) -> Result<Depth> {
        let d = Depth {
            total: self.total + 1,
            ..self
        };
        d.check()
    }

    fn check(self) -> Result<Depth> {
        if self.arrays > MAX_ARRAYS || self.structs > MAX_STRUCTS || self.total > MAX_DEPTH {
            return Err(Error::Invalid("values nest too deep"));
        }
        Ok(self)
    }
}

pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Writer {
        Writer { buf: Vec::new() }
    }

    pub fn pad(&mut self, align: usize) {
        while self.buf.len() % align != 0 {
            self.buf.push(0);
        }
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn u32(&mut self, v: u32) {
        self.pad(4);
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn string(&mut self, s: &str) -> Result<()> {
        if s.as_bytes().contains(&0) {
            return Err(Error::Invalid("string holds a NUL"));
        }
        let n = u32::try_from(s.len()).map_err(|_| Error::Invalid("string too long"))?;
        self.u32(n);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
        Ok(())
    }

    pub fn sig(&mut self, s: &str) -> Result<()> {
        sig::parse(s)?;
        self.buf.push(s.len() as u8);
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
        Ok(())
    }

    /// An array: the length, padding to the element, the elements.
    fn array(&mut self, elem: &Ty, f: impl FnOnce(&mut Writer) -> Result<()>) -> Result<()> {
        self.pad(4);
        let at = self.buf.len();
        self.buf.extend_from_slice(&[0; 4]);
        self.pad(elem.align());
        let start = self.buf.len();
        f(self)?;
        let len = self.buf.len() - start;
        if len > MAX_ARRAY {
            return Err(Error::Invalid("array too long"));
        }
        self.buf[at..at + 4].copy_from_slice(&(len as u32).to_le_bytes());
        Ok(())
    }

    pub fn value(&mut self, v: &Value) -> Result<()> {
        self.value_at(v, Depth::default())
    }

    fn value_at(&mut self, v: &Value, d: Depth) -> Result<()> {
        match v {
            Value::Byte(n) => self.u8(*n),
            Value::Bool(b) => self.u32(u32::from(*b)),
            Value::I16(n) => self.fixed(&n.to_le_bytes()),
            Value::U16(n) => self.fixed(&n.to_le_bytes()),
            Value::I32(n) => self.fixed(&n.to_le_bytes()),
            Value::U32(n) => self.fixed(&n.to_le_bytes()),
            Value::I64(n) => self.fixed(&n.to_le_bytes()),
            Value::U64(n) => self.fixed(&n.to_le_bytes()),
            Value::F64(n) => self.fixed(&n.to_le_bytes()),
            Value::Str(s) => self.string(s)?,
            Value::Path(s) => {
                if !names::path(s) {
                    return Err(Error::Invalid("bad object path"));
                }
                self.string(s)?;
            }
            Value::Sig(s) => self.sig(s)?,
            Value::Fd(n) => self.fixed(&n.to_le_bytes()),
            Value::Bytes(b) => {
                d.array()?;
                self.array(&Ty::Byte, |w| {
                    w.buf.extend_from_slice(b);
                    Ok(())
                })?;
            }
            Value::Array(elem, items) => {
                let d = d.array()?;
                self.array(elem, |w| {
                    for item in items {
                        if &item.ty() != elem {
                            return Err(Error::Invalid("array element of the wrong type"));
                        }
                        w.value_at(item, d)?;
                    }
                    Ok(())
                })?;
            }
            Value::Struct(fields) => {
                if fields.is_empty() {
                    return Err(Error::Invalid("empty struct"));
                }
                let d = d.strukt()?;
                self.pad(8);
                for f in fields {
                    self.value_at(f, d)?;
                }
            }
            Value::Entry(k, val) => {
                if !k.ty().is_basic() {
                    return Err(Error::Invalid("dict key is not basic"));
                }
                let d = d.strukt()?;
                self.pad(8);
                self.value_at(k, d)?;
                self.value_at(val, d)?;
            }
            Value::Variant(inner) => {
                let d = d.variant()?;
                self.sig(&inner.ty().sig())?;
                self.value_at(inner, d)?;
            }
        }
        Ok(())
    }

    fn fixed(&mut self, bytes: &[u8]) {
        self.pad(bytes.len());
        self.buf.extend_from_slice(bytes);
    }
}

pub struct Reader<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
    pub big: bool,
    /// How many fds came with the message, for `h` indexes.
    pub fds: u32,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8], big: bool) -> Reader<'a> {
        Reader {
            buf,
            pos: 0,
            big,
            fds: 0,
        }
    }

    pub fn align(&mut self, a: usize) -> Result<()> {
        let to = self.pos.div_ceil(a) * a;
        let pad = self
            .buf
            .get(self.pos..to)
            .ok_or(Error::Invalid("message ends in padding"))?;
        if pad.iter().any(|&b| b != 0) {
            return Err(Error::Invalid("padding is not zero"));
        }
        self.pos = to;
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(Error::Invalid("length overflows"))?;
        let s = self
            .buf
            .get(self.pos..end)
            .ok_or(Error::Invalid("message ends early"))?;
        self.pos = end;
        Ok(s)
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.align(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        if self.big {
            a.reverse();
        }
        Ok(a)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.fixed()?))
    }

    fn string(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        let s = self.take(n)?;
        if self.take(1)? != [0] {
            return Err(Error::Invalid("string not NUL terminated"));
        }
        if s.contains(&0) {
            return Err(Error::Invalid("string holds a NUL"));
        }
        String::from_utf8(s.to_vec()).map_err(|_| Error::Invalid("string is not UTF-8"))
    }

    pub fn sig(&mut self) -> Result<String> {
        let n = self.u8()? as usize;
        let s = self.take(n)?;
        if self.take(1)? != [0] {
            return Err(Error::Invalid("signature not NUL terminated"));
        }
        let s = std::str::from_utf8(s).map_err(|_| Error::Invalid("bad signature"))?;
        sig::parse(s)?;
        Ok(s.to_owned())
    }

    pub fn value(&mut self, ty: &Ty) -> Result<Value> {
        self.value_at(ty, Depth::default())
    }

    fn value_at(&mut self, ty: &Ty, d: Depth) -> Result<Value> {
        Ok(match ty {
            Ty::Byte => Value::Byte(self.u8()?),
            Ty::Bool => match self.u32()? {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                _ => return Err(Error::Invalid("boolean not 0 or 1")),
            },
            Ty::I16 => Value::I16(i16::from_le_bytes(self.fixed()?)),
            Ty::U16 => Value::U16(u16::from_le_bytes(self.fixed()?)),
            Ty::I32 => Value::I32(i32::from_le_bytes(self.fixed()?)),
            Ty::U32 => Value::U32(self.u32()?),
            Ty::I64 => Value::I64(i64::from_le_bytes(self.fixed()?)),
            Ty::U64 => Value::U64(u64::from_le_bytes(self.fixed()?)),
            Ty::F64 => Value::F64(f64::from_le_bytes(self.fixed()?)),
            Ty::Str => Value::Str(self.string()?),
            Ty::Path => {
                let s = self.string()?;
                if !names::path(&s) {
                    return Err(Error::Invalid("bad object path"));
                }
                Value::Path(s)
            }
            Ty::Sig => Value::Sig(self.sig()?),
            Ty::Fd => {
                let n = self.u32()?;
                if n >= self.fds {
                    return Err(Error::Invalid("fd index out of range"));
                }
                Value::Fd(n)
            }
            Ty::Array(elem) => {
                let d = d.array()?;
                let n = self.u32()? as usize;
                if n > MAX_ARRAY {
                    return Err(Error::Invalid("array too long"));
                }
                self.align(elem.align())?;
                let end = self
                    .pos
                    .checked_add(n)
                    .ok_or(Error::Invalid("length overflows"))?;
                if end > self.buf.len() {
                    return Err(Error::Invalid("array past the message"));
                }
                if **elem == Ty::Byte {
                    return Ok(Value::Bytes(self.take(n)?.to_vec()));
                }
                let mut items = Vec::new();
                while self.pos < end {
                    items.push(self.value_at(elem, d)?);
                }
                if self.pos != end {
                    return Err(Error::Invalid("array length mismatch"));
                }
                Value::Array((**elem).clone(), items)
            }
            Ty::Struct(fields) => {
                let d = d.strukt()?;
                self.align(8)?;
                let mut out = Vec::with_capacity(fields.len());
                for f in fields {
                    out.push(self.value_at(f, d)?);
                }
                Value::Struct(out)
            }
            Ty::Entry(k, v) => {
                let d = d.strukt()?;
                self.align(8)?;
                let k = self.value_at(k, d)?;
                let v = self.value_at(v, d)?;
                Value::Entry(Box::new(k), Box::new(v))
            }
            Ty::Variant => {
                let d = d.variant()?;
                let s = self.sig()?;
                let inner = Ty::parse_one(&s)?;
                Value::Variant(Box::new(self.value_at(&inner, d)?))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round(v: Value) {
        let mut w = Writer::new();
        w.value(&v).unwrap();
        let mut r = Reader::new(&w.buf, false);
        r.fds = 4;
        assert_eq!(r.value(&v.ty()).unwrap(), v);
        assert_eq!(r.pos, w.buf.len());
    }

    #[test]
    fn values_round_trip() {
        round(Value::Byte(7));
        round(Value::Bool(true));
        round(Value::I16(-3));
        round(Value::U64(u64::MAX));
        round(Value::F64(1.5));
        round(Value::str("héllo"));
        round(Value::path("/org/freedesktop/secrets"));
        round(Value::Sig("a{sv}".into()));
        round(Value::Fd(3));
        round(Value::Bytes(vec![1, 2, 3]));
        round(Value::props([
            ("Label", Value::str("x")),
            ("Locked", Value::Bool(false)),
        ]));
        round(Value::Struct(vec![
            Value::path("/s/1"),
            Value::Bytes(vec![]),
            Value::Bytes(b"pw".to_vec()),
            Value::str("text/plain"),
        ]));
        round(Value::Array(
            Ty::Array(Box::new(Ty::Str)),
            vec![Value::strs(["a"]), Value::strs::<&str>([])],
        ));
        round(Value::variant(Value::variant(Value::U32(1))));
    }

    #[test]
    fn big_endian_reads() {
        // u32 1, then string "ab".
        let buf = [0, 0, 0, 1, 0, 0, 0, 2, b'a', b'b', 0];
        let mut r = Reader::new(&buf, true);
        assert_eq!(r.value(&Ty::U32).unwrap(), Value::U32(1));
        assert_eq!(r.value(&Ty::Str).unwrap(), Value::str("ab"));
    }

    #[test]
    fn bad_input_is_an_error() {
        let mut r = Reader::new(&[2, 0, 0, 0], false);
        assert!(r.value(&Ty::Bool).is_err());
        let mut r = Reader::new(&[1, 0, 0, 0, b'a', 1], false);
        assert!(r.value(&Ty::Str).is_err());
        let mut r = Reader::new(&[255, 255, 255, 255], false);
        assert!(r.value(&Ty::Array(Box::new(Ty::U32))).is_err());
        let mut r = Reader::new(&[0, 0, 0, 0], false);
        assert!(r.value(&Ty::Fd).is_err(), "no fds came");
        // A u32 after a byte with junk padding.
        let mut r = Reader::new(&[1, 9, 0, 0, 1, 0, 0, 0], false);
        r.value(&Ty::Byte).unwrap();
        assert!(r.value(&Ty::U32).is_err());
    }

    #[test]
    fn deep_variants_stop() {
        let mut v = Value::U32(0);
        for _ in 0..70 {
            v = Value::variant(v);
        }
        assert!(Writer::new().value(&v).is_err());
        // Hand-built: 70 nested variants of "v".
        let mut buf = Vec::new();
        for _ in 0..70 {
            buf.extend_from_slice(&[1, b'v', 0]);
        }
        assert!(Reader::new(&buf, false).value(&Ty::Variant).is_err());
    }

    #[test]
    fn arrays_check_their_elements() {
        let bad = Value::Array(Ty::Str, vec![Value::U32(1)]);
        assert!(Writer::new().value(&bad).is_err());
    }
}
