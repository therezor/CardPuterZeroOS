//! `.desktop` files, read the way stock APPLaunch reads them
//! (`desktop_entry.cpp`, `cp0_desktop_exec_policy.cpp`), so an app built for
//! stock shows up here the same. Additions: `Comment` (the list layout's second
//! line) and `TryExec` (hide an entry whose program is missing).

use std::path::Path;

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
}

fn truthy(v: &str) -> bool {
    matches!(v, "true" | "True" | "1")
}

fn has_control(s: &str) -> bool {
    s.bytes().any(|b| b < 0x20 || b == 0x7f)
}

/// Parses the `[Desktop Entry]` group. `None` for hidden entries, a `Type`
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
            "Hidden" | "NoDisplay" => hidden |= truthy(&v),
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
            '%' => {
                if chars.next() == Some('%') {
                    cur.push('%');
                    in_word = true;
                }
            }
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
        assert!(parse("[Desktop Entry]\nName=A\nExec=/bin/a\nNoDisplay=true\n").is_none());
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
    fn filenames_must_be_plain_desktop_files() {
        assert!(filename_valid("zclaw.desktop"));
        assert!(!filename_valid(".desktop"));
        assert!(!filename_valid("a/b.desktop"));
        assert!(!filename_valid("notes.txt"));
    }
}
