//! `gh0st open PATH|URI`, also run as `xdg-open`: opens a file or a folder
//! in the app its type opens with, through the portal's OpenURI, as Files'
//! Enter does. Run from the terminal, the terminal is the app in front, so
//! the shell lets it ask.

use gh0st_dbus::{Conn, Kind, Message, Value, bus};
use std::path::Path;

const NAME: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";

pub fn main(args: &[String]) -> i32 {
    let [target] = args else {
        eprintln!("usage: gh0st open <file|folder|file:// URI>");
        return 2;
    };
    let uri = if target.starts_with("file://") {
        target.clone()
    } else {
        let p = Path::new(target);
        let abs = match std::fs::canonicalize(p) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("gh0st open: {target}: {e}");
                return 1;
            }
        };
        gh0st_core::uri::from_path(&abs)
    };
    let mut conn = match Conn::session() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gh0st open: no session bus: {e}");
            return 1;
        }
    };
    let me = conn.name.clone().unwrap_or_default();
    let token = format!("open{}", std::process::id());
    let path = format!(
        "{PATH}/request/{}/{token}",
        me.trim_start_matches(':').replace('.', "_")
    );
    let rule = format!(
        "type='signal',interface='org.freedesktop.portal.Request',member='Response',path='{path}'"
    );
    if let Err(e) = bus::add_match(&mut conn, &rule) {
        eprintln!("gh0st open: {e}");
        return 1;
    }
    let m = Message::call(NAME, PATH, "org.freedesktop.portal.OpenURI", "OpenURI")
        .arg(Value::str(""))
        .arg(Value::str(uri))
        .arg(Value::props([("handle_token", Value::str(token))]));
    if let Err(e) = conn.call(m, 5000) {
        eprintln!("gh0st open: {e}");
        return 1;
    }
    // The answer comes once the app is up, or at once if it can't be.
    let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < end {
        let mut fds = [gh0st_core::rt::pollfd(conn.fd())];
        let _ = gh0st_core::rt::wait(&mut fds, Some(gh0st_core::rt::now_ms() + 1000));
        if conn.fill().is_err() || conn.closed() {
            break;
        }
        while let Ok(Some(r)) = conn.recv() {
            if r.kind == Kind::Signal && r.path.as_deref() == Some(path.as_str()) {
                return match r.body.first().and_then(Value::as_u32) {
                    Some(0) => 0,
                    _ => {
                        eprintln!("gh0st open: refused: only the app in front may open files");
                        1
                    }
                };
            }
        }
    }
    0
}
