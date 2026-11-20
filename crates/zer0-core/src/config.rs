//! The shared device config, `~/.config/cardputerzero/config.json`. Stock apps
//! read it too (the camera app reads `camera.resolution.*`, every stock app
//! restores `brightness` and `volume` from it at start), so the format is
//! theirs: nested objects seen as flat dotted keys. Keys we do not know are
//! kept as they were. Our own keys start with `zer0.`.

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
        let map = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| parse(&s))
            .unwrap_or_default();
        Config {
            path: path.to_path_buf(),
            map,
        }
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

    pub fn set_int(&mut self, key: &str, v: i64) {
        self.map.insert(key.to_string(), Value::Num(v.to_string()));
    }

    pub fn set_str(&mut self, key: &str, v: &str) {
        self.map.insert(key.to_string(), Value::Str(v.to_string()));
    }

    pub fn remove(&mut self, key: &str) {
        self.map.remove(key);
    }

    /// Writes atomically: a temp file in the same directory, fsync, rename.
    /// A crash leaves the old file or the new one, never half of either.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(&self.path, to_json(&self.map).as_bytes())
    }
}

pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

/// Parses a JSON object into dotted keys. `None` if the text is not an object.
pub fn parse(text: &str) -> Option<BTreeMap<String, Value>> {
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
                                self.i += 6;
                                cp = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
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
        let dir = std::env::temp_dir().join(format!("zer0-config-{}", std::process::id()));
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, STOCK).unwrap();
        let mut c = Config::load(&path);
        assert_eq!(c.get_int("brightness"), Some(80));
        c.set_str("zer0.home_layout", "list");
        c.set_int("brightness", 60);
        c.save().unwrap();
        let c = Config::load(&path);
        assert_eq!(c.get_str("zer0.home_layout"), Some("list"));
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
}
