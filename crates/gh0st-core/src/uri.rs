//! `file://` URIs, for `%u` field codes and the portal: every byte outside
//! RFC 3986's unreserved set and `/` is percent-encoded, so any path survives.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

fn keep(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/')
}

/// `/a b` as `file:///a%20b`.
pub fn from_path(p: &Path) -> String {
    let mut s = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        if keep(b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

fn hex(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|d| d as u8)
}

/// The path a `file://` URI names: local only (empty host or `localhost`),
/// absolute, with no NUL. Anything else is `None`.
pub fn to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut out = Vec::with_capacity(rest.len());
    let mut b = rest.bytes();
    while let Some(c) = b.next() {
        if c == b'%' {
            out.push(hex(b.next()?)? << 4 | hex(b.next()?)?);
        } else {
            out.push(c);
        }
    }
    if out.contains(&0) {
        return None;
    }
    Some(PathBuf::from(OsStr::from_bytes(&out)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_path_round_trips() {
        let odd = PathBuf::from(OsStr::from_bytes(b"/home/u/a b%c\t\xff\xe9.txt"));
        let u = from_path(&odd);
        assert_eq!(u, "file:///home/u/a%20b%25c%09%FF%E9.txt");
        assert_eq!(to_path(&u), Some(odd));
    }

    #[test]
    fn only_local_absolute_paths() {
        assert_eq!(to_path("file://localhost/etc/x"), Some("/etc/x".into()));
        assert_eq!(to_path("file://host/etc/x"), None);
        assert_eq!(to_path("http://a/b"), None);
        assert_eq!(to_path("file:///a%00b"), None);
        assert_eq!(to_path("file:///a%zz"), None);
    }
}
