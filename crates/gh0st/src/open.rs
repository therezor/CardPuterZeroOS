//! The portal from the terminal. Run there, the terminal is the app in
//! front, so the shell lets it ask.
//!
//! - `gh0st open PATH|URI`, also run as `xdg-open`: opens a file or a
//!   folder in the app its type opens with, through OpenURI, as Files'
//!   Enter does.
//! - `gh0st screenshot`: the screen, after the user's yes. Prints the file.
//! - `gh0st pick-color`: a colour picked from the screen, as `#RRGGBB`.
//! - `gh0st wallpaper FILE`: the wallpaper, after the user's yes.

use gh0st_dbus::{Conn, Kind, Message, Value, bus};
use std::path::Path;

const NAME: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
/// How long a question may wait for the user.
const USER_SECS: u64 = 600;

/// Calls `iface.member(args.., {handle_token})` and waits for its
/// Response: the code and the results.
fn ask(iface: &str, member: &str, args: Vec<Value>, secs: u64) -> Result<(u32, Value), String> {
    let mut conn = Conn::session().map_err(|e| format!("no session bus: {e}"))?;
    let me = conn.name.clone().unwrap_or_default();
    let token = format!("cli{}", std::process::id());
    let path = format!(
        "{PATH}/request/{}/{token}",
        me.trim_start_matches(':').replace('.', "_")
    );
    let rule = format!(
        "type='signal',interface='org.freedesktop.portal.Request',member='Response',path='{path}'"
    );
    bus::add_match(&mut conn, &rule).map_err(|e| e.to_string())?;
    let mut m = Message::call(NAME, PATH, iface, member);
    m.body = args;
    let m = m.arg(Value::props([("handle_token", Value::str(token))]));
    conn.call(m, 5000).map_err(|e| e.to_string())?;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        let mut fds = [gh0st_core::rt::pollfd(conn.fd())];
        let _ = gh0st_core::rt::wait(&mut fds, Some(gh0st_core::rt::now_ms() + 1000));
        if conn.fill().is_err() || conn.closed() {
            break;
        }
        while let Ok(Some(r)) = conn.recv() {
            if r.kind == Kind::Signal && r.path.as_deref() == Some(path.as_str()) {
                let code = r.body.first().and_then(Value::as_u32).unwrap_or(2);
                let results = r.body.get(1).cloned().unwrap_or(Value::props::<&str>([]));
                return Ok((code, results));
            }
        }
    }
    Err("no answer".into())
}

/// Prints why a request ended without a result.
fn refused(cmd: &str, code: u32) -> i32 {
    match code {
        1 => eprintln!("gh0st {cmd}: cancelled"),
        _ => eprintln!("gh0st {cmd}: refused: only the app in front may ask"),
    }
    1
}

fn uri_of(cmd: &str, target: &str) -> Option<String> {
    if target.starts_with("file://") {
        return Some(target.to_string());
    }
    match std::fs::canonicalize(Path::new(target)) {
        Ok(a) => Some(gh0st_core::uri::from_path(&a)),
        Err(e) => {
            eprintln!("gh0st {cmd}: {target}: {e}");
            None
        }
    }
}

pub fn main(args: &[String]) -> i32 {
    let [target] = args else {
        eprintln!("usage: gh0st open <file|folder|file:// URI>");
        return 2;
    };
    let Some(uri) = uri_of("open", target) else {
        return 1;
    };
    // The answer comes once the app is up, or at once if it can't be.
    let args = vec![Value::str(""), Value::str(uri)];
    match ask("org.freedesktop.portal.OpenURI", "OpenURI", args, 30) {
        Ok((0, _)) => 0,
        Ok((code, _)) => refused("open", code),
        Err(e) => {
            eprintln!("gh0st open: {e}");
            1
        }
    }
}

pub fn screenshot(args: &[String]) -> i32 {
    if !args.is_empty() {
        eprintln!("usage: gh0st screenshot");
        return 2;
    }
    let iface = "org.freedesktop.portal.Screenshot";
    match ask(iface, "Screenshot", vec![Value::str("")], USER_SECS) {
        Ok((0, r)) => {
            let uri = r.get("uri").map(Value::inner).and_then(Value::as_str);
            match uri.and_then(gh0st_core::uri::to_path) {
                Some(p) => {
                    println!("{}", p.display());
                    0
                }
                None => 1,
            }
        }
        Ok((code, _)) => refused("screenshot", code),
        Err(e) => {
            eprintln!("gh0st screenshot: {e}");
            1
        }
    }
}

pub fn pick_color(args: &[String]) -> i32 {
    if !args.is_empty() {
        eprintln!("usage: gh0st pick-color");
        return 2;
    }
    let iface = "org.freedesktop.portal.Screenshot";
    match ask(iface, "PickColor", vec![Value::str("")], USER_SECS) {
        Ok((0, r)) => match r.get("color").map(Value::inner) {
            Some(Value::Struct(c)) if c.len() == 3 => {
                let ch: Vec<u8> = c
                    .iter()
                    .map(|v| match v {
                        Value::F64(f) => (f.clamp(0.0, 1.0) * 255.0).round() as u8,
                        _ => 0,
                    })
                    .collect();
                println!("#{:02X}{:02X}{:02X}", ch[0], ch[1], ch[2]);
                0
            }
            _ => 1,
        },
        Ok((code, _)) => refused("pick-color", code),
        Err(e) => {
            eprintln!("gh0st pick-color: {e}");
            1
        }
    }
}

pub fn wallpaper(args: &[String]) -> i32 {
    let [target] = args else {
        eprintln!("usage: gh0st wallpaper <picture>");
        return 2;
    };
    let Some(uri) = uri_of("wallpaper", target) else {
        return 1;
    };
    let iface = "org.freedesktop.portal.Wallpaper";
    let args = vec![Value::str(""), Value::str(uri)];
    match ask(iface, "SetWallpaperURI", args, USER_SECS) {
        Ok((0, _)) => 0,
        Ok((code, _)) => refused("wallpaper", code),
        Err(e) => {
            eprintln!("gh0st wallpaper: {e}");
            1
        }
    }
}
