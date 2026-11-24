//! Mutation fuzzing: real messages, bytes flipped, cut and grown. The
//! decoder must never panic, and anything it accepts must survive a round
//! trip. Deterministic, so a failure reproduces.

use std::collections::VecDeque;

use gh0st_dbus::sig::Ty;
use gh0st_dbus::{Message, Value};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn corpus() -> Vec<Vec<u8>> {
    let mut calls = [
        Message::call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "Hello",
        ),
        Message::call(
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
            "OpenSession",
        )
        .arg(Value::str("plain"))
        .arg(Value::variant(Value::str(""))),
        Message::call(
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets/collection/login",
            "org.freedesktop.Secret.Collection",
            "CreateItem",
        )
        .arg(Value::props([
            ("org.freedesktop.Secret.Item.Label", Value::str("git")),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Value::str_dict([("service", "github.com"), ("user", "x")]),
            ),
        ]))
        .arg(Value::Struct(vec![
            Value::path("/org/freedesktop/secrets/session/1"),
            Value::Bytes(vec![]),
            Value::Bytes(b"hunter2".to_vec()),
            Value::str("text/plain"),
        ]))
        .arg(Value::Bool(true)),
        Message::call(
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "Notify",
        )
        .arg(Value::str("app"))
        .arg(Value::U32(0))
        .arg(Value::str("icon"))
        .arg(Value::str("Title"))
        .arg(Value::str("Body"))
        .arg(Value::strs(["default", "Open"]))
        .arg(Value::props([("urgency", Value::Byte(1))]))
        .arg(Value::I32(-1)),
        Message::signal("/a", "a.b", "C").arg(Value::Array(
            Ty::Array(Box::new(Ty::Variant)),
            vec![Value::Array(
                Ty::Variant,
                vec![Value::variant(Value::F64(2.0))],
            )],
        )),
    ];
    let mut out = Vec::new();
    for (i, m) in calls.iter_mut().enumerate() {
        m.sender = Some(":1.5".into());
        out.push(m.encode(i as u32 + 1).unwrap());
    }
    out
}

#[test]
fn mutated_messages_never_panic() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let seeds = corpus();
    for m in &seeds {
        assert!(Message::decode(m, &mut VecDeque::new()).is_ok());
    }
    let mut accepted = 0;
    for _ in 0..200_000 {
        let mut m = seeds[rng.below(seeds.len())].clone();
        for _ in 0..1 + rng.below(4) {
            match rng.below(5) {
                0 => {
                    let i = rng.below(m.len());
                    m[i] = rng.next() as u8;
                }
                1 => {
                    let i = rng.below(m.len());
                    m[i] ^= 1 << rng.below(8);
                }
                2 => m.truncate(rng.below(m.len()) + 1),
                3 => {
                    let i = rng.below(m.len());
                    m.insert(i, rng.next() as u8);
                }
                _ => {
                    // Lengths are where parsers break: hit a word with an edge.
                    let i = rng.below(m.len().saturating_sub(4).max(1));
                    let v = [0u32, 1, 0x7f, 0xff, 0xffff, 0x7fff_ffff, u32::MAX][rng.below(7)];
                    if i + 4 <= m.len() {
                        m[i..i + 4].copy_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }
        let _ = Message::frame_len(&m);
        if let Ok(msg) = Message::decode(&m, &mut VecDeque::new()) {
            accepted += 1;
            let again = msg
                .encode(msg.serial)
                .expect("an accepted message re-encodes");
            let back = Message::decode(&again, &mut VecDeque::new()).expect("and decodes");
            assert_eq!(back.body, msg.body);
            assert_eq!(back.member, msg.member);
        }
    }
    assert!(accepted > 0, "some mutations are still valid");
}
