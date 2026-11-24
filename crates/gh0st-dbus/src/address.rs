//! Bus addresses: `unix:path=` and, on Linux, `unix:abstract=`.

use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Addr {
    Path(PathBuf),
    Abstract(Vec<u8>),
}

/// The first address we can use from a `;` separated list.
pub fn parse(s: &str) -> Option<Addr> {
    s.split(';').find_map(|one| {
        let rest = one.strip_prefix("unix:")?;
        rest.split(',').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            let v = unescape(v)?;
            match k {
                "path" => Some(Addr::Path(PathBuf::from(String::from_utf8(v).ok()?))),
                "abstract" => Some(Addr::Abstract(v)),
                _ => None,
            }
        })
    })
}

fn unescape(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

pub fn session() -> Option<Addr> {
    if let Ok(a) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        return parse(&a);
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(Addr::Path(PathBuf::from(dir).join("bus")))
}

pub fn system() -> Option<Addr> {
    match std::env::var("DBUS_SYSTEM_BUS_ADDRESS") {
        Ok(a) => parse(&a),
        Err(_) => Some(Addr::Path("/run/dbus/system_bus_socket".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_parse() {
        assert_eq!(
            parse("unix:path=/run/user/1000/bus"),
            Some(Addr::Path("/run/user/1000/bus".into()))
        );
        assert_eq!(
            parse("tcp:host=x;unix:abstract=/tmp/dbus-x%2Cy,guid=12"),
            Some(Addr::Abstract(b"/tmp/dbus-x,y".to_vec()))
        );
        assert_eq!(parse("tcp:host=x"), None);
        assert_eq!(parse("unix:path=%zz"), None);
    }
}
