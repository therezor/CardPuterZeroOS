//! The portal's `org.freedesktop.portal.Settings`: the look apps should
//! follow, from the shared config (`gh0st.palette`, `gh0st.animations`).
//! Light or dark, the accent and reduced motion, under
//! `org.freedesktop.appearance`. A change to config.json sends
//! `SettingChanged` for each key that moved.

use gh0st_core::config::Config;
use gh0st_dbus::bus::err;
use gh0st_dbus::{Message, Ty, Value};
use gh0st_ui::{color, theme};
use std::path::{Path, PathBuf};

pub const IFACE: &str = "org.freedesktop.portal.Settings";
pub const VERSION: u32 = 2;
const PATH: &str = "/org/freedesktop/portal/desktop";
const NS: &str = "org.freedesktop.appearance";
const NOT_FOUND: &str = "org.freedesktop.portal.Error.NotFound";

#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    /// 1 dark, 2 light.
    pub scheme: u32,
    /// The palette's accent, each channel 0 to 1.
    pub accent: (f64, f64, f64),
    /// 0: no preference.
    pub contrast: u32,
    /// 1: animations are off.
    pub motion: u32,
}

impl Appearance {
    pub fn of(config: &Config) -> Appearance {
        let p = theme::named(config.palette_name());
        let [r, g, b] = color::to_rgb8(p.accent);
        Appearance {
            scheme: if color::luminance(p.bg) > 0.5 { 2 } else { 1 },
            accent: (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0),
            contrast: 0,
            motion: if config.animations() { 0 } else { 1 },
        }
    }

    fn get(&self, key: &str) -> Option<Value> {
        Some(match key {
            "color-scheme" => Value::U32(self.scheme),
            "accent-color" => Value::Struct(vec![
                Value::F64(self.accent.0),
                Value::F64(self.accent.1),
                Value::F64(self.accent.2),
            ]),
            "contrast" => Value::U32(self.contrast),
            "reduced-motion" => Value::U32(self.motion),
            _ => return None,
        })
    }

    const KEYS: [&'static str; 4] = ["color-scheme", "accent-color", "contrast", "reduced-motion"];
}

pub struct Settings {
    config: PathBuf,
    now: Appearance,
    /// Signals to send.
    pub out: Vec<Message>,
}

/// A namespace pattern as ReadAll takes them: exact, or ending in `*`.
fn wanted(patterns: &[Value]) -> bool {
    patterns.is_empty()
        || patterns
            .iter()
            .filter_map(Value::as_str)
            .any(|p| match p.strip_suffix('*') {
                Some(prefix) => NS.starts_with(prefix),
                None => p == NS || p.is_empty(),
            })
}

impl Settings {
    pub fn new(config: &Path) -> Settings {
        Settings {
            config: config.to_path_buf(),
            now: Appearance::of(&Config::load(config)),
            out: Vec::new(),
        }
    }

    pub fn call(&mut self, m: &Message) -> Message {
        let s = |i: usize| m.body.get(i).and_then(Value::as_str).unwrap_or("");
        match m.member.as_deref().unwrap_or("") {
            "ReadAll" => {
                let pats = m.body.first().and_then(Value::as_array).unwrap_or(&[]);
                let entries = if wanted(pats) {
                    vec![(
                        Value::str(NS),
                        Value::props(
                            Appearance::KEYS
                                .iter()
                                .filter_map(|k| self.now.get(k).map(|v| (*k, v))),
                        ),
                    )]
                } else {
                    Vec::new()
                };
                Message::reply(m).arg(Value::dict(
                    Ty::Str,
                    Ty::Array(Box::new(Ty::Entry(
                        Box::new(Ty::Str),
                        Box::new(Ty::Variant),
                    ))),
                    entries,
                ))
            }
            // Read wraps the value twice, as the old version did.
            "Read" | "ReadOne" => match (s(0) == NS).then(|| self.now.get(s(1))).flatten() {
                Some(v) if m.member.as_deref() == Some("Read") => {
                    Message::reply(m).arg(Value::variant(Value::variant(v)))
                }
                Some(v) => Message::reply(m).arg(Value::variant(v)),
                None => Message::error(m, NOT_FOUND, "no such setting"),
            },
            _ => Message::error(m, err::UNKNOWN_METHOD, "no such method"),
        }
    }

    /// config.json changed: a SettingChanged for each key that moved.
    pub fn changed(&mut self) {
        let next = Appearance::of(&Config::load(&self.config));
        for k in Appearance::KEYS {
            let (old, new) = (self.now.get(k), next.get(k));
            if old != new {
                if let Some(v) = new {
                    self.out.push(
                        Message::signal(PATH, IFACE, "SettingChanged")
                            .arg(Value::str(NS))
                            .arg(Value::str(k))
                            .arg(Value::variant(v)),
                    );
                }
            }
        }
        self.now = next;
    }
}

pub const XML: &str = r#"<interface name="org.freedesktop.portal.Settings"><method name="ReadAll"><arg type="as" direction="in"/><arg type="a{sa{sv}}" direction="out"/></method><method name="Read"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method><method name="ReadOne"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method><signal name="SettingChanged"><arg type="s"/><arg type="s"/><arg type="v"/></signal><property name="version" type="u" access="read"/></interface>"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn config(dir: &Path, json: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join("config.json");
        std::fs::write(&p, json).unwrap();
        p
    }

    fn call(s: &mut Settings, member: &str, args: Vec<Value>) -> Message {
        let mut m = Message::call("x", PATH, IFACE, member);
        m.body = args;
        m.serial = 1;
        s.call(&m)
    }

    #[test]
    fn the_palette_says_light_or_dark_and_a_change_is_signalled() {
        let dir =
            std::env::temp_dir().join(format!("gh0st-settings-portal-{}", std::process::id()));
        let p = config(
            &dir,
            r#"{"gh0st": {"palette": "Mint", "animations": true}}"#,
        );
        let mut s = Settings::new(&p);
        let r = call(
            &mut s,
            "ReadOne",
            vec![Value::str(NS), Value::str("color-scheme")],
        );
        assert_eq!(r.body[0].inner(), &Value::U32(1));
        let all = call(&mut s, "ReadAll", vec![Value::strs(["org.freedesktop.*"])]);
        assert!(all.body[0].get(NS).is_some());
        let none = call(&mut s, "ReadAll", vec![Value::strs(["org.kde.*"])]);
        assert!(none.body[0].get(NS).is_none());
        assert!(
            call(&mut s, "ReadOne", vec![Value::str(NS), Value::str("font")])
                .error_name
                .is_some()
        );
        config(
            &dir,
            r#"{"gh0st": {"palette": "Paper", "animations": false}}"#,
        );
        s.changed();
        let keys: Vec<&str> = s.out.iter().filter_map(|m| m.body[1].as_str()).collect();
        assert!(
            keys.contains(&"color-scheme") && keys.contains(&"reduced-motion"),
            "{keys:?}"
        );
        let scheme = s
            .out
            .iter()
            .find(|m| m.body[1].as_str() == Some("color-scheme"))
            .unwrap();
        assert_eq!(scheme.body[2].inner(), &Value::U32(2));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
