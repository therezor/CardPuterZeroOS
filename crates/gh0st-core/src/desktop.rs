//! `.desktop` files, read the way stock APPLaunch reads them
//! (`desktop_entry.cpp`, `cp0_desktop_exec_policy.cpp`), so an app built for
//! stock shows up here the same. Additions: `Comment` (the list layout's second
//! line) and `TryExec` (hide an entry whose program is missing).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const MAX_NAME: usize = 128;
pub const MAX_ICON: usize = 512;
pub const MAX_EXEC: usize = 512;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DesktopEntry {
    pub name: String,
    pub exec: String,
    pub icon: String,
    pub comment: String,
    pub try_exec: String,
    pub terminal: bool,
    /// Stock: after a terminal app exits, wait for a key before leaving.
    pub sysplause: bool,
    /// `X-Gh0st-Background`: the app runs on behind other apps unless the
    /// user says otherwise. Honoured only for an app on the shell link.
    pub background: bool,
    /// `MimeType=`: the file types the app opens.
    pub mime_types: Vec<String>,
    /// `NoDisplay`: not on home or in Settings > Apps, but it may still
    /// open files.
    pub no_display: bool,
}

/// At most this many `MimeType=` entries are kept.
pub const MAX_MIME_TYPES: usize = 64;

/// How an `Exec` takes files, from its field code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldCode {
    /// `%f`: one path.
    File,
    /// `%F`: any number of paths.
    Files,
    /// `%u`: one URI.
    Url,
    /// `%U`: any number of URIs.
    Urls,
}

impl FieldCode {
    fn of(c: char) -> Option<FieldCode> {
        match c {
            'f' => Some(FieldCode::File),
            'F' => Some(FieldCode::Files),
            'u' => Some(FieldCode::Url),
            'U' => Some(FieldCode::Urls),
            _ => None,
        }
    }

    pub fn many(self) -> bool {
        matches!(self, FieldCode::Files | FieldCode::Urls)
    }

    /// The words for `files` in this code's form: paths or `file://` URIs,
    /// all of them or the first. Paths keep every byte.
    pub fn args(self, files: &[PathBuf]) -> Vec<OsString> {
        let files = if self.many() {
            files
        } else {
            &files[..files.len().min(1)]
        };
        files
            .iter()
            .map(|f| match self {
                FieldCode::File | FieldCode::Files => f.clone().into_os_string(),
                FieldCode::Url | FieldCode::Urls => crate::uri::from_path(f).into(),
            })
            .collect()
    }
}

fn truthy(v: &str) -> bool {
    matches!(v, "true" | "True" | "1")
}

fn has_control(s: &str) -> bool {
    s.bytes().any(|b| b < 0x20 || b == 0x7f)
}

/// Parses the `[Desktop Entry]` group. `None` for `Hidden` entries, a `Type`
/// other than `Application`, a missing or oversized `Name` or `Exec`, or
/// control characters where stock rejects them.
pub fn parse(contents: &str) -> Option<DesktopEntry> {
    let mut e = DesktopEntry {
        sysplause: true,
        ..Default::default()
    };
    let mut in_entry = false;
    let mut hidden = false;
    let mut app_type = true;
    for line in contents.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (
            k.trim_matches([' ', '\t']),
            v.trim_matches([' ', '\t']).to_string(),
        );
        match k {
            "Name" => e.name = v,
            "Exec" => e.exec = v,
            "Icon" => e.icon = v,
            "Comment" => e.comment = v,
            "TryExec" => e.try_exec = v,
            "Terminal" => e.terminal = truthy(&v),
            "Sysplause" => e.sysplause = truthy(&v),
            "X-Gh0st-Background" => e.background = truthy(&v),
            "MimeType" => {
                e.mime_types = v
                    .split(';')
                    .map(str::trim)
                    .filter(|m| m.contains('/') && m.len() <= 255 && !has_control(m))
                    .take(MAX_MIME_TYPES)
                    .map(String::from)
                    .collect()
            }
            "NoDisplay" => e.no_display = truthy(&v),
            "Hidden" => hidden |= truthy(&v),
            "Type" => app_type = v == "Application",
            _ => {}
        }
    }
    let ok = !hidden
        && app_type
        && !e.name.is_empty()
        && !e.exec.is_empty()
        && e.name.len() <= MAX_NAME
        && e.icon.len() <= MAX_ICON
        && e.exec.len() <= MAX_EXEC
        && !has_control(&e.name)
        && !has_control(&e.icon)
        && !e.exec.contains('\0');
    if has_control(&e.comment) {
        e.comment.clear();
    }
    ok.then_some(e)
}

/// `*.desktop`, no path separators or control characters.
pub fn filename_valid(name: &str) -> bool {
    name.len() > ".desktop".len()
        && name.ends_with(".desktop")
        && !name
            .bytes()
            .any(|b| b < 0x20 || b == 0x7f || b == b'/' || b == b'\\')
}

/// Bare program names stock lets an `Exec` start with. Anything else must be
/// an absolute path to an executable file.
const ALLOWED_NAMES: &[&str] = &["bash", "python3", "vim", "vi", "nano", "sh"];

/// The stock Exec rule. `executable` checks a path, so tests need no files.
pub fn exec_allowed(exec: &str, executable: impl Fn(&Path) -> bool) -> Result<(), &'static str> {
    if exec.is_empty() {
        return Err("empty Exec");
    }
    if exec.len() > MAX_EXEC {
        return Err("Exec too long");
    }
    if exec.contains(['|', '&', ';', '<', '>', '`', '$', '\\', '\n', '\r']) {
        return Err("Exec contains shell metacharacters");
    }
    let first = exec.split_whitespace().next().ok_or("missing executable")?;
    if first.contains('/') {
        return if executable(Path::new(first)) {
            Ok(())
        } else {
            Err("executable path is not executable")
        };
    }
    if ALLOWED_NAMES.contains(&first) {
        Ok(())
    } else {
        Err("executable name is not allowlisted")
    }
}

/// A regular file someone may execute.
pub fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Splits an `Exec` into argv the desktop-entry way: double quotes group, a
/// backslash escapes inside them, and field codes (`%f`, `%U`...) drop out.
/// Used only for our own entries; third-party ones go through `sh -c`, as in
/// stock, because the stock rule lets them rely on `~` and quoting.
pub fn split_exec(exec: &str) -> Vec<String> {
    expand_exec(exec, &[])
        .into_iter()
        .map(|w| w.to_string_lossy().into_owned())
        .collect()
}

/// The words of an `Exec`. A field code stays in its word as `\u{1}` and the
/// code's letter, for [`expand_exec`] to fill.
fn split_words(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_word = true;
            }
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if in_word {
                    args.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '%' => match chars.next() {
                Some('%') => {
                    cur.push('%');
                    in_word = true;
                }
                Some(n) => {
                    cur.push('\u{1}');
                    cur.push(n);
                    in_word = true;
                }
                None => {}
            },
            c => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        args.push(cur);
    }
    args.retain(|a| !a.is_empty());
    args
}

/// The field code that takes files in `exec`, outside single quotes. The
/// first one counts, as the spec allows only one.
pub fn field_code(exec: &str) -> Option<FieldCode> {
    let mut single = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => single = !single,
            '%' if !single => match chars.next() {
                Some(n) => {
                    if let Some(code) = FieldCode::of(n) {
                        return Some(code);
                    }
                }
                None => break,
            },
            _ => {}
        }
    }
    None
}

/// [`split_exec`] with `files` put where the field code is: a word that is
/// only `%F` or `%U` becomes one word per file, any other code is replaced
/// in place by the first file. Without a code, `files` are not passed.
pub fn expand_exec(exec: &str, files: &[PathBuf]) -> Vec<OsString> {
    let code = field_code(exec);
    let mut out = Vec::new();
    for word in split_words(exec) {
        match (word.as_str(), code) {
            ("\u{1}F" | "\u{1}U", Some(c)) => out.extend(c.args(files)),
            _ => {
                let mut w = OsString::new();
                let mut chars = word.chars();
                while let Some(ch) = chars.next() {
                    if ch != '\u{1}' {
                        w.push(ch.encode_utf8(&mut [0; 4]));
                    } else if let (Some(n), Some(c)) = (chars.next(), code) {
                        if FieldCode::of(n).is_some() {
                            w.push(c.args(files).join(" ".as_ref()));
                        }
                    }
                }
                if !w.is_empty() {
                    out.push(w);
                }
            }
        }
    }
    out
}

/// A third-party `Exec` as the line `sh -c` runs. The field code becomes
/// `"$1"` or `"$@"`, so the files travel as `sh`'s positional arguments and
/// never inside the line. Inside double quotes the code becomes `$1` or `$@`,
/// which the quotes already guard. `exec_allowed` bans `$`, so nothing else
/// in the line can expand.
pub fn shell_line(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len() + 8);
    let (mut single, mut double) = (false, false);
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' if !double => {
                single = !single;
                out.push(c);
            }
            '"' if !single => {
                double = !double;
                out.push(c);
            }
            '%' if !single => match chars.next() {
                Some('%') => out.push('%'),
                // Deprecated and other codes (%i, %c, %k) drop out.
                Some(n) => {
                    if let Some(code) = FieldCode::of(n) {
                        let var = if code.many() { "$@" } else { "$1" };
                        if double {
                            out.push_str(var);
                        } else {
                            out.push('"');
                            out.push_str(var);
                            out.push('"');
                        }
                    }
                }
                None => {}
            },
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK_ZCLAW: &str = "[Desktop Entry]\nName=ZClaw\nExec=/usr/share/APPLaunch/bin/ZClaw\nTerminal=false\nIcon=share/images/claw_100.png\nType=Application\n";

    #[test]
    fn a_stock_entry_parses_with_the_stock_defaults() {
        let e = parse(STOCK_ZCLAW).unwrap();
        assert_eq!(e.name, "ZClaw");
        assert_eq!(e.icon, "share/images/claw_100.png");
        assert!(!e.terminal);
        assert!(e.sysplause, "Sysplause defaults to true");
    }

    #[test]
    fn hidden_wrong_type_and_missing_exec_are_dropped() {
        assert!(parse("[Desktop Entry]\nName=A\nExec=/bin/a\nHidden=true\n").is_none());
        assert!(parse("[Desktop Entry]\nName=A\nExec=/bin/a\nType=Link\n").is_none());
        assert!(parse("[Desktop Entry]\nName=A\n").is_none());
        assert!(parse("[Other]\nName=A\nExec=/bin/a\n").is_none());
        let long = format!("[Desktop Entry]\nName={}\nExec=/bin/a\n", "x".repeat(129));
        assert!(parse(&long).is_none());
    }

    #[test]
    fn keys_outside_the_entry_group_are_ignored() {
        let s = "[Desktop Entry]\nName=A\nExec=/bin/a\n[Desktop Action x]\nName=B\nExec=/bin/b\n";
        let e = parse(s).unwrap();
        assert_eq!((e.name.as_str(), e.exec.as_str()), ("A", "/bin/a"));
    }

    #[test]
    fn the_exec_rule_matches_stock() {
        let yes = |_: &Path| true;
        let no = |_: &Path| false;
        assert!(exec_allowed("/usr/bin/app --x", yes).is_ok());
        assert!(exec_allowed("/usr/bin/app", no).is_err());
        assert!(exec_allowed("python3 ~/game.py", no).is_ok());
        assert!(exec_allowed("htop", yes).is_err());
        assert!(exec_allowed("/bin/a; rm -rf /", yes).is_err());
        assert!(exec_allowed("/bin/a $HOME", yes).is_err());
    }

    #[test]
    fn exec_splits_into_argv_without_field_codes() {
        assert_eq!(
            split_exec("/usr/bin/gh0st settings"),
            ["/usr/bin/gh0st", "settings"]
        );
        assert_eq!(split_exec("app \"two words\" %U"), ["app", "two words"]);
        assert_eq!(
            split_exec("app \"a \\\"q\\\"\" 100%%"),
            ["app", "a \"q\"", "100%"]
        );
    }

    #[test]
    fn no_display_entries_are_kept_for_opening_files() {
        let e = parse("[Desktop Entry]\nName=A\nExec=/bin/a %f\nNoDisplay=true\nMimeType=text/plain; image/*;bad;\n").unwrap();
        assert!(e.no_display);
        assert_eq!(e.mime_types, ["text/plain", "image/*"]);
    }

    #[test]
    fn field_codes_take_files_where_they_stand() {
        let f = |e: &str| field_code(e);
        assert_eq!(f("/bin/a %f"), Some(FieldCode::File));
        assert_eq!(f("/bin/a --open=%U"), Some(FieldCode::Urls));
        assert_eq!(f("/bin/a 100%%"), None);
        assert_eq!(f("/bin/a '%f'"), None);
        assert_eq!(f("/bin/a %i"), None);
        let files = [PathBuf::from("/home/u/a b.txt"), PathBuf::from("/tmp/c")];
        assert_eq!(
            expand_exec("/bin/a %F x", &files),
            ["/bin/a", "/home/u/a b.txt", "/tmp/c", "x"]
        );
        assert_eq!(
            expand_exec("/bin/a %f", &files),
            ["/bin/a", "/home/u/a b.txt"]
        );
        assert_eq!(
            expand_exec("/bin/a --in=%f", &files[1..]),
            ["/bin/a", "--in=/tmp/c"]
        );
        assert_eq!(
            expand_exec("/bin/a %U", &files[..1]),
            ["/bin/a", "file:///home/u/a%20b.txt"]
        );
        assert_eq!(expand_exec("/bin/a %f", &[]), ["/bin/a"]);
        assert_eq!(expand_exec("/bin/a", &files), ["/bin/a"]);
    }

    #[test]
    fn a_shell_line_takes_files_as_positional_arguments() {
        assert_eq!(shell_line("python3 ~/v.py %f"), "python3 ~/v.py \"$1\"");
        assert_eq!(shell_line("/opt/a \"%F\" 5%%"), "/opt/a \"$@\" 5%");
        assert_eq!(shell_line("/opt/a %i %u"), "/opt/a  \"$1\"");
        assert_eq!(shell_line("/opt/a '%f'"), "/opt/a '%f'");
    }

    #[test]
    fn filenames_must_be_plain_desktop_files() {
        assert!(filename_valid("zclaw.desktop"));
        assert!(!filename_valid(".desktop"));
        assert!(!filename_valid("a/b.desktop"));
        assert!(!filename_valid("notes.txt"));
    }
}
