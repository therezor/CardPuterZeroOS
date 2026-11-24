//! A tiny Secret Service client for checks on a device, where no
//! secret-tool is installed. One connection for the whole run:
//!
//!     krtool store LABEL ATTR VALUE SECRET
//!     krtool lookup ATTR VALUE      (unlocks through a prompt if it must)
//!
//! Run it under two names and it is two apps to the keyring.

use gh0st_dbus::{Conn, Kind, Message, Value, bus};
use std::time::{Duration, Instant};

const S: &str = "org.freedesktop.secrets";
const R: &str = "/org/freedesktop/secrets";
const SVC: &str = "org.freedesktop.Secret.Service";

fn call(c: &mut Conn, path: &str, iface: &str, member: &str, args: Vec<Value>) -> Vec<Value> {
    let mut m = Message::call(S, path, iface, member);
    m.body = args;
    match c.call(m, 5000) {
        Ok(r) => r.body,
        Err(e) => {
            eprintln!("{member}: {e}");
            std::process::exit(1)
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut c = Conn::session().expect("session bus");
    bus::add_match(
        &mut c,
        "type='signal',interface='org.freedesktop.Secret.Prompt'",
    )
    .ok();
    let sess = call(
        &mut c,
        R,
        SVC,
        "OpenSession",
        vec![Value::str("plain"), Value::variant(Value::str(""))],
    )[1]
    .as_str()
    .unwrap_or("/")
    .to_string();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["store", label, k, v, secret] => {
            let props = Value::props([
                ("org.freedesktop.Secret.Item.Label", Value::str(*label)),
                (
                    "org.freedesktop.Secret.Item.Attributes",
                    Value::str_dict([(*k, *v)]),
                ),
            ]);
            let sec = Value::Struct(vec![
                Value::path(sess),
                Value::Bytes(vec![]),
                Value::Bytes(secret.as_bytes().to_vec()),
                Value::str("text/plain"),
            ]);
            let b = call(
                &mut c,
                &format!("{R}/aliases/default"),
                "org.freedesktop.Secret.Collection",
                "CreateItem",
                vec![props, sec, Value::Bool(true)],
            );
            println!("stored {}", b[0].as_str().unwrap_or("?"));
        }
        ["lookup", k, v] => {
            let b = call(
                &mut c,
                R,
                SVC,
                "SearchItems",
                vec![Value::str_dict([(*k, *v)])],
            );
            let list = |x: &Value| -> Vec<String> {
                x.as_array()
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(|p| p.as_str().map(String::from))
                    .collect()
            };
            let (open, locked) = (list(&b[0]), list(&b[1]));
            let item = match (open.first(), locked.first()) {
                (Some(i), _) => i.clone(),
                (None, Some(i)) => {
                    eprintln!("locked: asking");
                    let u = call(&mut c, R, SVC, "Unlock", vec![Value::paths([i.clone()])]);
                    let p = u[1].as_str().unwrap_or("/").to_string();
                    if p != "/" {
                        call(
                            &mut c,
                            &p,
                            "org.freedesktop.Secret.Prompt",
                            "Prompt",
                            vec![Value::str("")],
                        );
                        let end = Instant::now() + Duration::from_secs(60);
                        let dismissed = loop {
                            if Instant::now() > end {
                                eprintln!("no answer");
                                std::process::exit(2);
                            }
                            let _ = c.fill();
                            if let Ok(Some(m)) = c.recv() {
                                if m.kind == Kind::Signal
                                    && m.is("org.freedesktop.Secret.Prompt", "Completed")
                                {
                                    break m.body[0].as_bool().unwrap_or(true);
                                }
                            }
                            std::thread::sleep(Duration::from_millis(50));
                        };
                        if dismissed {
                            println!("denied");
                            return;
                        }
                    }
                    i.clone()
                }
                (None, None) => {
                    println!("not found");
                    std::process::exit(1);
                }
            };
            let mut m = Message::call(S, &item, "org.freedesktop.Secret.Item", "GetSecret");
            m.body = vec![Value::path(sess)];
            match c.call(m, 5000) {
                Ok(r) => println!(
                    "{}",
                    String::from_utf8_lossy(
                        r.body[0]
                            .as_struct()
                            .and_then(|f| f[2].as_bytes())
                            .unwrap_or_default()
                    )
                ),
                Err(e) => println!("refused: {e}"),
            }
        }
        _ => eprintln!("usage: krtool store LABEL ATTR VALUE SECRET | lookup ATTR VALUE"),
    }
}
