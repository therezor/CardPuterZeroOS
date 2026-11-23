//! The shared device config, `~/.config/cardputerzero/config.json`. Stock apps
//! read it too (the camera app reads `camera.resolution.*`, every stock app
//! restores `brightness` and `volume` from it at start), so the format is
//! theirs: nested objects seen as flat dotted keys. Keys we do not know are
//! kept as they were. Our own keys start with `gh0st.`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    /// Kept as written so `007` or `1e3` round-trip unchanged.
    Num(String),
    Bool(bool),
    Null,
    /// Arrays are not ours to edit; their JSON text is kept whole.
    Raw(String),
}

#[derive(Debug, Default)]
pub struct Config {
    path: PathBuf,
    map: BTreeMap<String, Value>,
    /// The file was there but did not parse. A save moves it aside first, so
    /// its keys are never lost to a write that holds only ours.
    broken: bool,
}

/// The keys more than one program reads. Each has one reader below, so the
/// shell and every app agree on its default.
pub mod key {
    pub const BRIGHTNESS: &str = "brightness";
    pub const VOLUME: &str = "volume";
    /// Seconds before the screen goes dark, as stock stores it. 0 is never.
    pub const DARK_TIME: &str = "dark_time";
    pub const PALETTE: &str = "gh0st.palette";
    /// `12h`, else 24-hour.
    pub const CLOCK: &str = "gh0st.clock";
    pub const ANIMATIONS: &str = "gh0st.animations";
    pub const HOME_LAYOUT: &str = "gh0st.home_layout";
    pub const LOCK_PASSWORD: &str = "gh0st.lock.password";
    /// Seconds dark before the lock; -1 locks only by hand.
    pub const LOCK_AFTER: &str = "gh0st.lock.after";
    /// Seconds dark before Wi-Fi goes off; absent or negative is never.
    pub const DARK_WIFI_OFF: &str = "gh0st.dark.wifi_off";
    /// The terminal home opens. Set by hand; none is the built-in one.
    pub const TERMINAL: &str = "gh0st.terminal";
    /// Where the Store downloads apps from: auto, github or china.
    pub const STORE_MIRROR: &str = "gh0st.store.mirror";
}

pub const DEFAULT_DARK_SECS: i64 = 30;
pub const DEFAULT_LOCK_AFTER_SECS: i64 = 300;
/// Volume keys and Settings' Sound row move this many percent.
pub const VOLUME_STEP: i32 = 10;

/// Seconds to milliseconds, capped so a hand-edited huge value can't wrap a
/// sum of times.
fn secs_ms(secs: i64) -> u64 {
    const MAX_SECS: u64 = 1 << 32;
    (secs.max(0) as u64).min(MAX_SECS) * 1000
}

pub fn default_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    home.join(".config/cardputerzero/config.json")
}

impl Config {
    /// Reads `path`. A missing or broken file gives an empty config: the shell
    /// must start with defaults rather than fail.
    pub fn load(path: &Path) -> Config {
        let (map, broken) = match std::fs::read(path) {
            Ok(bytes) => match std::str::from_utf8(&bytes).ok().and_then(parse) {
                Some(map) => (map, false),
                None => (BTreeMap::new(), true),
            },
            Err(_) => (BTreeMap::new(), false),
        };
        Config {
            path: path.to_path_buf(),
            map,
            broken,
        }
    }

    /// Reads the file again, applies `f` and saves. Every writer goes through
    /// here, so a key another program wrote meanwhile survives.
    pub fn update(path: &Path, f: impl FnOnce(&mut Config)) -> std::io::Result<Config> {
        let mut c = Config::load(path);
        f(&mut c);
        c.save()?;
        Ok(c)
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.map.get(key)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.map.get(key)? {
            Value::Str(s) | Value::Num(s) => Some(s),
            _ => None,
        }
    }

    pub fn get_int(&self, key: &str) -> Option<i64> {
        match self.map.get(key)? {
            Value::Num(s) | Value::Str(s) => s.trim().parse().ok(),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.map.get(key)? {
            Value::Bool(b) => Some(*b),
            Value::Num(s) | Value::Str(s) => match s.as_str() {
                "1" | "true" | "on" => Some(true),
                "0" | "false" | "off" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn clock_24h(&self) -> bool {
        self.get_str(key::CLOCK) != Some("12h")
    }

    pub fn palette_name(&self) -> Option<&str> {
        self.get_str(key::PALETTE)
    }

    pub fn animations(&self) -> bool {
        self.get_bool(key::ANIMATIONS).unwrap_or(true)
    }

    pub fn dark_secs(&self) -> i64 {
        self.get_int(key::DARK_TIME).unwrap_or(DEFAULT_DARK_SECS)
    }

    /// When the screen goes dark; None is never.
    pub fn dark_ms(&self) -> Option<u64> {
        let s = self.dark_secs();
        (s > 0).then(|| secs_ms(s))
    }

    pub fn lock_password(&self) -> bool {
        self.get_bool(key::LOCK_PASSWORD).unwrap_or(true)
    }

    pub fn lock_after_secs(&self) -> i64 {
        self.get_int(key::LOCK_AFTER)
            .unwrap_or(DEFAULT_LOCK_AFTER_SECS)
    }

    /// How long dark before the lock; None locks only by hand.
    pub fn lock_after_ms(&self) -> Option<u64> {
        let s = self.lock_after_secs();
        (s >= 0).then(|| secs_ms(s))
    }

    pub fn dark_wifi_secs(&self) -> Option<i64> {
        self.get_int(key::DARK_WIFI_OFF).filter(|&s| s >= 0)
    }

    pub fn dark_wifi_ms(&self) -> Option<u64> {
        self.dark_wifi_secs().map(secs_ms)
    }

    pub fn set_int(&mut self, key: &str, v: i64) {
        self.map.insert(key.to_string(), Value::Num(v.to_string()));
    }

    pub fn set_bool(&mut self, key: &str, v: bool) {
        self.map.insert(key.to_string(), Value::Bool(v));
    }

    pub fn set_str(&mut self, key: &str, v: &str) {
        self.map.insert(key.to_string(), Value::Str(v.to_string()));
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn remove(&mut self, key: &str) {
        self.map.remove(key);
    }

    /// Writes atomically: a temp file in the same directory, fsync, rename.
    /// A crash leaves the old file or the new one, never half of either.
    /// A file that would not parse is kept as `config.json.broken`.
    pub fn save(&mut self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if self.broken {
            let aside = self.path.with_extension("json.broken");
            eprintln!(
                "gh0st: {} did not parse, kept as {}",
                self.path.display(),
                aside.display()
            );
            std::fs::rename(&self.path, &aside)?;
            self.broken = false;
        }
        write_atomic(&self.path, to_json(&self.map).as_bytes())
    }
}

/// Writes through a temp file named for this process, so two programs saving
/// at once never write the same temp file, then fsync and rename.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let mut f = std::fs::File::create(&tmp)?;
    let r = f.write_all(data).and_then(|_| f.sync_all());
    drop(f);
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path)
}

/// Parses a JSON object into dotted keys. `None` if the text is not an object.
pub fn parse(text: &str) -> Option<BTreeMap<String, Value>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
    };
    p.ws();
    let mut map = BTreeMap::new();
    p.object("", &mut map)?;
    p.ws();
    (p.i == p.s.len()).then_some(map)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, b: u8) -> Option<()> {
        self.ws();
        (self.s.get(self.i) == Some(&b)).then(|| self.i += 1)
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn object(&mut self, prefix: &str, out: &mut BTreeMap<String, Value>) -> Option<()> {
        self.eat(b'{')?;
        if self.peek()? == b'}' {
            self.i += 1;
            // An empty object stays, so a save writes it back.
            if !prefix.is_empty() {
                out.insert(prefix.to_string(), Value::Raw("{}".into()));
            }
            return Some(());
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.eat(b':')?;
            let full = if prefix.is_empty() {
                key
            } else {
                format!("{prefix}.{key}")
            };
            match self.peek()? {
                b'{' => self.object(&full, out)?,
                _ => {
                    let v = self.value()?;
                    out.insert(full, v);
                }
            }
            match self.peek()? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    return Some(());
                }
                _ => return None,
            }
        }
    }

    fn value(&mut self) -> Option<Value> {
        match self.peek()? {
            b'"' => self.string().map(Value::Str),
            b'[' => {
                let start = self.i;
                self.skip_array()?;
                Some(Value::Raw(
                    String::from_utf8_lossy(&self.s[start..self.i]).into_owned(),
                ))
            }
            b't' => self.word("true").map(|_| Value::Bool(true)),
            b'f' => self.word("false").map(|_| Value::Bool(false)),
            b'n' => self.word("null").map(|_| Value::Null),
            _ => {
                let start = self.i;
                while self.i < self.s.len()
                    && matches!(
                        self.s[self.i],
                        b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
                    )
                {
                    self.i += 1;
                }
                (self.i > start).then(|| {
                    Value::Num(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
                })
            }
        }
    }

    fn word(&mut self, w: &str) -> Option<()> {
        self.s[self.i..]
            .starts_with(w.as_bytes())
            .then(|| self.i += w.len())
    }

    fn skip_array(&mut self) -> Option<()> {
        let mut depth = 0;
        while self.i < self.s.len() {
            match self.s[self.i] {
                b'"' => {
                    self.string()?;
                    continue;
                }
                b'[' | b'{' => depth += 1,
                b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.i += 1;
                        return Some(());
                    }
                }
                _ => {}
            }
            self.i += 1;
        }
        None
    }

    fn string(&mut self) -> Option<String> {
        if self.s.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out = Vec::new();
        while let Some(&b) = self.s.get(self.i) {
            self.i += 1;
            match b {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let hex = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                            self.i += 4;
                            let mut cp = u32::from_str_radix(hex, 16).ok()?;
                            if (0xd800..0xdc00).contains(&cp)
                                && self.s.get(self.i..self.i + 2) == Some(b"\\u")
                            {
                                let lo = std::str::from_utf8(self.s.get(self.i + 2..self.i + 6)?)
                                    .ok()?;
                                let lo = u32::from_str_radix(lo, 16).ok()?;
                                if (0xdc00..0xe000).contains(&lo) {
                                    self.i += 6;
                                    cp = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                                }
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                            out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                _ => out.push(b),
            }
        }
        None
    }
}

/// Rebuilds the nested objects from dotted keys, two-space indented.
pub fn to_json(map: &BTreeMap<String, Value>) -> String {
    #[derive(Default)]
    struct Node {
        leaf: Option<Value>,
        kids: BTreeMap<String, Node>,
    }
    let mut root = Node::default();
    for (k, v) in map {
        let mut n = &mut root;
        for part in k.split('.') {
            n = n.kids.entry(part.to_string()).or_default();
        }
        n.leaf = Some(v.clone());
    }
    fn emit(n: &Node, depth: usize, out: &mut String) {
        out.push('{');
        let mut first = true;
        for (k, kid) in &n.kids {
            out.push_str(if first { "\n" } else { ",\n" });
            first = false;
            out.push_str(&"  ".repeat(depth + 1));
            escape(k, out);
            out.push_str(": ");
            match &kid.leaf {
                Some(v) if kid.kids.is_empty() => value(v, out),
                // A key that is both a value and a parent cannot be nested;
                // keep the children, as stock does.
                _ => emit(kid, depth + 1, out),
            }
        }
        if !first {
            out.push('\n');
            out.push_str(&"  ".repeat(depth));
        }
        out.push('}');
    }
    fn value(v: &Value, out: &mut String) {
        match v {
            Value::Str(s) => escape(s, out),
            Value::Num(s) | Value::Raw(s) => out.push_str(s),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Null => out.push_str("null"),
        }
    }
    fn escape(s: &str, out: &mut String) {
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
    }
    let mut out = String::new();
    emit(&root, 0, &mut out);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK: &str = r#"{
      "brightness": 80,
      "dark_time": 30,
      "camera": { "resolution": { "width": 1280, "height": 720 } },
      "run_as_user": "pi",
      "app_Game": 0,
      "list": [1, 2, {"x": "]"}],
      "flag": true,
      "name": "café \"zero\""
    }"#;

    #[test]
    fn nested_objects_read_as_dotted_keys() {
        let map = parse(STOCK).unwrap();
        assert_eq!(map["camera.resolution.width"], Value::Num("1280".into()));
        assert_eq!(map["name"], Value::Str("café \"zero\"".into()));
        assert_eq!(map["list"], Value::Raw(r#"[1, 2, {"x": "]"}]"#.into()));
    }

    #[test]
    fn a_round_trip_keeps_every_key_we_do_not_know() {
        let map = parse(STOCK).unwrap();
        let again = parse(&to_json(&map)).unwrap();
        assert_eq!(map, again);
    }

    #[test]
    fn our_keys_go_in_beside_stock_ones() {
        let dir = std::env::temp_dir().join(format!("gh0st-config-{}", std::process::id()));
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, STOCK).unwrap();
        let mut c = Config::load(&path);
        assert_eq!(c.get_int("brightness"), Some(80));
        c.set_str("gh0st.home_layout", "list");
        c.set_int("brightness", 60);
        c.save().unwrap();
        let c = Config::load(&path);
        assert_eq!(c.get_str("gh0st.home_layout"), Some("list"));
        assert_eq!(c.get_int("brightness"), Some(60));
        assert_eq!(c.get_int("camera.resolution.height"), Some(720));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_broken_file_is_an_empty_config() {
        assert!(parse("{\"a\": ").is_none());
        assert!(parse("[1]").is_none());
        let c = Config::load(Path::new("/nonexistent/config.json"));
        assert_eq!(c.get_int("brightness"), None);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gh0st-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.json")
    }

    #[test]
    fn a_save_over_a_broken_file_keeps_it_aside() {
        let path = scratch("broken");
        std::fs::write(&path, "{\"brightness\": 80, \"camera\": ").unwrap();
        Config::update(&path, |c| c.set_int("volume", 40)).unwrap();
        let kept = std::fs::read_to_string(path.with_extension("json.broken")).unwrap();
        assert!(kept.contains("\"brightness\": 80"));
        assert_eq!(Config::load(&path).get_int("volume"), Some(40));
        // The next save finds a good file and moves nothing.
        Config::update(&path, |c| c.set_int("volume", 50)).unwrap();
        let again = std::fs::read_to_string(path.with_extension("json.broken")).unwrap();
        assert_eq!(kept, again);
    }

    #[test]
    fn update_keeps_a_key_written_since_the_last_load() {
        let path = scratch("stale");
        std::fs::write(&path, STOCK).unwrap();
        Config::update(&path, |c| c.set_str("gh0st.palette", "Nord")).unwrap();
        let c = Config::update(&path, |c| c.set_str("gh0st.home_layout", "list")).unwrap();
        assert_eq!(c.get_str("gh0st.palette"), Some("Nord"));
        assert_eq!(c.get_int("camera.resolution.width"), Some(1280));
    }

    #[test]
    fn a_byte_order_mark_and_empty_objects_survive() {
        let text = "\u{feff}{\"a\": 1, \"plugins\": {}, \"b\": {\"c\": {}}}";
        let map = parse(text).unwrap();
        assert_eq!(map["a"], Value::Num("1".into()));
        let again = parse(&to_json(&map)).unwrap();
        assert_eq!(map, again);
        assert!(to_json(&map).contains("\"plugins\": {}"));
    }

    #[test]
    fn a_lone_surrogate_is_a_replacement_char() {
        let map = parse(r#"{"s": "\ud800\u0041", "t": "\ud83d\ude00"}"#).unwrap();
        assert_eq!(map["s"], Value::Str("\u{fffd}A".into()));
        assert_eq!(map["t"], Value::Str("\u{1f600}".into()));
    }

    #[test]
    fn writers_use_their_own_temp_file() {
        let path = scratch("tmp");
        write_atomic(&path, b"{}").unwrap();
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("config.json")]);
    }

    #[test]
    fn typed_keys_have_one_default_and_never_wrap() {
        let mut c = Config::default();
        assert!(c.clock_24h() && c.animations() && c.lock_password());
        assert_eq!(c.dark_ms(), Some(30_000));
        assert_eq!(c.lock_after_ms(), Some(300_000));
        assert_eq!(c.dark_wifi_ms(), None);
        c.set_str(key::CLOCK, "12h");
        c.set_int(key::DARK_TIME, 0);
        c.set_int(key::LOCK_AFTER, -1);
        c.set_int(key::DARK_WIFI_OFF, -5);
        assert!(!c.clock_24h());
        assert_eq!(
            (c.dark_ms(), c.lock_after_ms(), c.dark_wifi_ms()),
            (None, None, None)
        );
        c.set_int(key::LOCK_AFTER, i64::MAX);
        let a = c.lock_after_ms().unwrap();
        assert!(
            a.checked_add(u64::MAX / 2).is_some(),
            "a sum of times stays in range"
        );
    }
}
