//! Asks the file portal for a file and prints what came back: a client the
//! way libportal is one, for trying the portal on a device.
//!
//!     pick [open|save|folder]

use gh0st_dbus::{Conn, Kind, Message, Value, bus};

fn main() {
    let what = std::env::args().nth(1).unwrap_or_else(|| "open".into());
    let mut c = Conn::session().expect("session bus");
    let me = c.name.clone().unwrap_or_default();
    let path = format!(
        "/org/freedesktop/portal/desktop/request/{}/pick1",
        me.trim_start_matches(':').replace('.', "_")
    );
    let rule = format!(
        "type='signal',interface='org.freedesktop.portal.Request',member='Response',path='{path}'"
    );
    bus::add_match(&mut c, &rule).expect("match");
    let mut opts = vec![("handle_token", Value::str("pick1"))];
    if what == "folder" {
        opts.push(("directory", Value::Bool(true)));
    }
    if what == "save" {
        opts.push(("current_name", Value::str("new.txt")));
    }
    let member = if what == "save" { "SaveFile" } else { "OpenFile" };
    let m = Message::call(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.FileChooser",
        member,
    )
    .arg(Value::str(""))
    .arg(Value::str("Pick"))
    .arg(Value::props(opts));
    let r = c.call(m, 3000).expect("call");
    println!("request {:?}", r.body.first());
    loop {
        let mut fds = [libc::pollfd {
            fd: c.fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        // SAFETY: one pollfd on our own connection.
        unsafe { libc::poll(fds.as_mut_ptr(), 1, -1) };
        c.fill().expect("read");
        while let Ok(Some(m)) = c.recv() {
            if m.kind == Kind::Signal && m.is("org.freedesktop.portal.Request", "Response") {
                let code = m.body.first().and_then(Value::as_u32);
                let uris = m.body.get(1).and_then(|r| r.get("uris")).map(Value::inner).cloned();
                println!("response {code:?} {uris:?}");
                return;
            }
        }
    }
}
