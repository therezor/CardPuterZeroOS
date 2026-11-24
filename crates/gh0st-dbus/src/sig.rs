//! Type signatures: parsed into a tree once, checked against the spec's
//! limits (255 bytes, 32 nested arrays, 32 nested structs).

use crate::{Error, Result};

pub const MAX_SIG: usize = 255;
pub const MAX_ARRAYS: usize = 32;
pub const MAX_STRUCTS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ty {
    Byte,
    Bool,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F64,
    Str,
    Path,
    Sig,
    Fd,
    Variant,
    Array(Box<Ty>),
    Struct(Vec<Ty>),
    /// A dict entry: only ever an array's element.
    Entry(Box<Ty>, Box<Ty>),
}

impl Ty {
    pub fn align(&self) -> usize {
        match self {
            Ty::Byte | Ty::Sig | Ty::Variant => 1,
            Ty::I16 | Ty::U16 => 2,
            Ty::Bool | Ty::I32 | Ty::U32 | Ty::Str | Ty::Path | Ty::Fd | Ty::Array(_) => 4,
            Ty::I64 | Ty::U64 | Ty::F64 | Ty::Struct(_) | Ty::Entry(..) => 8,
        }
    }

    pub fn is_basic(&self) -> bool {
        !matches!(
            self,
            Ty::Variant | Ty::Array(_) | Ty::Struct(_) | Ty::Entry(..)
        )
    }

    pub fn write(&self, out: &mut String) {
        let c = match self {
            Ty::Byte => 'y',
            Ty::Bool => 'b',
            Ty::I16 => 'n',
            Ty::U16 => 'q',
            Ty::I32 => 'i',
            Ty::U32 => 'u',
            Ty::I64 => 'x',
            Ty::U64 => 't',
            Ty::F64 => 'd',
            Ty::Str => 's',
            Ty::Path => 'o',
            Ty::Sig => 'g',
            Ty::Fd => 'h',
            Ty::Variant => 'v',
            Ty::Array(e) => {
                out.push('a');
                e.write(out);
                return;
            }
            Ty::Struct(fields) => {
                out.push('(');
                for f in fields {
                    f.write(out);
                }
                out.push(')');
                return;
            }
            Ty::Entry(k, v) => {
                out.push('{');
                k.write(out);
                v.write(out);
                out.push('}');
                return;
            }
        };
        out.push(c);
    }

    pub fn sig(&self) -> String {
        let mut s = String::new();
        self.write(&mut s);
        s
    }

    /// One complete type, such as a variant's.
    pub fn parse_one(sig: &str) -> Result<Ty> {
        let mut tys = parse(sig)?;
        if tys.len() != 1 {
            return Err(Error::Invalid("not one complete type"));
        }
        Ok(tys.remove(0))
    }
}

/// A sequence of complete types, such as a message body's.
pub fn parse(sig: &str) -> Result<Vec<Ty>> {
    let b = sig.as_bytes();
    if b.len() > MAX_SIG {
        return Err(Error::Invalid("signature too long"));
    }
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        out.push(one(b, &mut i, 0, 0)?);
    }
    Ok(out)
}

fn one(b: &[u8], i: &mut usize, arrays: usize, structs: usize) -> Result<Ty> {
    let Some(&c) = b.get(*i) else {
        return Err(Error::Invalid("signature ends early"));
    };
    *i += 1;
    Ok(match c {
        b'y' => Ty::Byte,
        b'b' => Ty::Bool,
        b'n' => Ty::I16,
        b'q' => Ty::U16,
        b'i' => Ty::I32,
        b'u' => Ty::U32,
        b'x' => Ty::I64,
        b't' => Ty::U64,
        b'd' => Ty::F64,
        b's' => Ty::Str,
        b'o' => Ty::Path,
        b'g' => Ty::Sig,
        b'h' => Ty::Fd,
        b'v' => Ty::Variant,
        b'a' => {
            if arrays + 1 > MAX_ARRAYS {
                return Err(Error::Invalid("arrays nest too deep"));
            }
            if b.get(*i) == Some(&b'{') {
                *i += 1;
                if structs + 1 > MAX_STRUCTS {
                    return Err(Error::Invalid("structs nest too deep"));
                }
                let k = one(b, i, arrays + 1, structs + 1)?;
                if !k.is_basic() {
                    return Err(Error::Invalid("dict key is not basic"));
                }
                let v = one(b, i, arrays + 1, structs + 1)?;
                if b.get(*i) != Some(&b'}') {
                    return Err(Error::Invalid("dict entry not closed"));
                }
                *i += 1;
                Ty::Array(Box::new(Ty::Entry(Box::new(k), Box::new(v))))
            } else {
                Ty::Array(Box::new(one(b, i, arrays + 1, structs)?))
            }
        }
        b'(' => {
            if structs + 1 > MAX_STRUCTS {
                return Err(Error::Invalid("structs nest too deep"));
            }
            let mut fields = Vec::new();
            loop {
                match b.get(*i) {
                    Some(b')') => {
                        *i += 1;
                        break;
                    }
                    Some(_) => fields.push(one(b, i, arrays, structs + 1)?),
                    None => return Err(Error::Invalid("struct not closed")),
                }
            }
            if fields.is_empty() {
                return Err(Error::Invalid("empty struct"));
            }
            Ty::Struct(fields)
        }
        _ => return Err(Error::Invalid("bad type code")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_round_trip() {
        for s in [
            "",
            "s",
            "a{sv}",
            "(oayays)",
            "a{oa{sa{sv}}}",
            "aay",
            "a(yv)",
            "h",
        ] {
            let tys = parse(s).unwrap();
            let back: String = tys.iter().map(Ty::sig).collect();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn bad_signatures_are_refused() {
        for s in [
            "a", "(", "()", "a{vs}", "a{s}", "a{sss}", "{ss}", "z", "(s", "a{ss",
        ] {
            assert!(parse(s).is_err(), "{s} should fail");
        }
        assert!(parse(&"a".repeat(33).chars().chain(['y']).collect::<String>()).is_err());
        assert!(parse(&format!("{}y{}", "(".repeat(33), ")".repeat(33))).is_err());
        assert!(parse(&"y".repeat(256)).is_err());
        assert!(parse(&format!("{}y", "a".repeat(32))).is_ok());
    }

    #[test]
    fn one_complete_type() {
        assert!(Ty::parse_one("s").is_ok());
        assert!(Ty::parse_one("ss").is_err());
        assert!(Ty::parse_one("").is_err());
    }
}
