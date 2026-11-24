//! Values in a message body. An `ay` is always [`Value::Bytes`], so a
//! secret is one buffer that [`Value::wipe`] can zero.

use crate::sig::Ty;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(u8),
    Bool(bool),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(String),
    Path(String),
    Sig(String),
    /// An index into the message's fds.
    Fd(u32),
    Bytes(Vec<u8>),
    /// The element type, then the elements.
    Array(Ty, Vec<Value>),
    Struct(Vec<Value>),
    Entry(Box<Value>, Box<Value>),
    Variant(Box<Value>),
}

impl Value {
    pub fn ty(&self) -> Ty {
        match self {
            Value::Byte(_) => Ty::Byte,
            Value::Bool(_) => Ty::Bool,
            Value::I16(_) => Ty::I16,
            Value::U16(_) => Ty::U16,
            Value::I32(_) => Ty::I32,
            Value::U32(_) => Ty::U32,
            Value::I64(_) => Ty::I64,
            Value::U64(_) => Ty::U64,
            Value::F64(_) => Ty::F64,
            Value::Str(_) => Ty::Str,
            Value::Path(_) => Ty::Path,
            Value::Sig(_) => Ty::Sig,
            Value::Fd(_) => Ty::Fd,
            Value::Bytes(_) => Ty::Array(Box::new(Ty::Byte)),
            Value::Array(e, _) => Ty::Array(Box::new(e.clone())),
            Value::Struct(f) => Ty::Struct(f.iter().map(Value::ty).collect()),
            Value::Entry(k, v) => Ty::Entry(Box::new(k.ty()), Box::new(v.ty())),
            Value::Variant(_) => Ty::Variant,
        }
    }

    pub fn str(s: impl Into<String>) -> Value {
        Value::Str(s.into())
    }

    pub fn path(s: impl Into<String>) -> Value {
        Value::Path(s.into())
    }

    pub fn variant(v: Value) -> Value {
        Value::Variant(Box::new(v))
    }

    /// An array of object paths, the common `ao`.
    pub fn paths<S: Into<String>>(items: impl IntoIterator<Item = S>) -> Value {
        Value::Array(Ty::Path, items.into_iter().map(Value::path).collect())
    }

    pub fn strs<S: Into<String>>(items: impl IntoIterator<Item = S>) -> Value {
        Value::Array(Ty::Str, items.into_iter().map(Value::str).collect())
    }

    /// A dict from its key and value types and its pairs.
    pub fn dict(k: Ty, v: Ty, pairs: impl IntoIterator<Item = (Value, Value)>) -> Value {
        Value::Array(
            Ty::Entry(Box::new(k), Box::new(v)),
            pairs
                .into_iter()
                .map(|(k, v)| Value::Entry(Box::new(k), Box::new(v)))
                .collect(),
        )
    }

    /// The common `a{sv}`.
    pub fn props<K: Into<String>>(pairs: impl IntoIterator<Item = (K, Value)>) -> Value {
        Value::dict(
            Ty::Str,
            Ty::Variant,
            pairs
                .into_iter()
                .map(|(k, v)| (Value::str(k), Value::variant(v))),
        )
    }

    /// The common `a{ss}`.
    pub fn str_dict<K: Into<String>, V: Into<String>>(
        pairs: impl IntoIterator<Item = (K, V)>,
    ) -> Value {
        Value::dict(
            Ty::Str,
            Ty::Str,
            pairs
                .into_iter()
                .map(|(k, v)| (Value::str(k), Value::str(v))),
        )
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Path(s) | Value::Sig(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match *self {
            Value::U32(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        match *self {
            Value::I32(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(_, items) => Some(items),
            _ => None,
        }
    }

    pub fn as_struct(&self) -> Option<&[Value]> {
        match self {
            Value::Struct(f) => Some(f),
            _ => None,
        }
    }

    /// The value inside a variant, through any number of them.
    pub fn inner(&self) -> &Value {
        match self {
            Value::Variant(v) => v.inner(),
            v => v,
        }
    }

    /// The pairs of a dict; empty for anything else.
    pub fn entries(&self) -> impl Iterator<Item = (&Value, &Value)> {
        self.as_array()
            .unwrap_or(&[])
            .iter()
            .filter_map(|e| match e {
                Value::Entry(k, v) => Some((&**k, &**v)),
                _ => None,
            })
    }

    /// The value under string key `key` in a dict.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries()
            .find(|(k, _)| k.as_str() == Some(key))
            .map(|(_, v)| v)
    }

    /// Zeroes every byte and string this value holds.
    pub fn wipe(&mut self) {
        match self {
            Value::Str(s) | Value::Path(s) | Value::Sig(s) => {
                // SAFETY: zero bytes keep the string valid UTF-8.
                crate::wipe(unsafe { s.as_bytes_mut() });
            }
            Value::Bytes(b) => crate::wipe(b),
            Value::Array(_, items) | Value::Struct(items) => items.iter_mut().for_each(Value::wipe),
            Value::Entry(k, v) => {
                k.wipe();
                v.wipe();
            }
            Value::Variant(v) => v.wipe(),
            _ => {}
        }
    }
}
