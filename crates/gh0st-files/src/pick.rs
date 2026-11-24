//! Files as the file chooser: what an app asked for through the portal
//! (`docs/portal.md`), and the filters it named.

use gh0st_core::mime;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Pick one file, or several when `multiple`.
    Open,
    /// Pick a name in a folder to save to.
    Save,
    /// Pick a folder.
    Folder,
}

/// One pattern of a filter, as the portal gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    /// `*.txt`: `*`, `?` and `[...]`, any case.
    Glob(String),
    /// `image/png` or `image/*`.
    Mime(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub patterns: Vec<Pattern>,
}

impl Filter {
    pub fn matches(&self, file: &str) -> bool {
        self.patterns.iter().any(|p| match p {
            Pattern::Glob(g) => glob(g, file),
            Pattern::Mime(m) => mime::of_name(file).is_some_and(|t| mime::is_a(t, m)),
        })
    }
}

/// What the asking app wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub mode: Mode,
    pub multiple: bool,
    /// The app asking, by name: "Text".
    pub app: String,
    /// Its word for the main action, when it gave one.
    pub accept: Option<String>,
    pub filters: Vec<Filter>,
    /// The filter in force, an index into `filters`.
    pub filter: Option<usize>,
    /// The name a save starts with.
    pub name: String,
    /// Where to start; home when None.
    pub folder: Option<PathBuf>,
}

impl Spec {
    pub fn new(mode: Mode, app: &str) -> Spec {
        Spec {
            mode,
            multiple: false,
            app: app.into(),
            accept: None,
            filters: Vec::new(),
            filter: None,
            name: String::new(),
            folder: None,
        }
    }

    /// The main action's word: the app's, or Open, Save, Choose.
    pub fn verb(&self) -> &str {
        match (&self.accept, self.mode) {
            (Some(a), _) => a,
            (None, Mode::Open) => "Open",
            (None, Mode::Save) => "Save",
            (None, Mode::Folder) => "Choose",
        }
    }

    /// Whether a file shows: folders always do, files only to Open and
    /// Save, through the filter in force.
    pub fn shows(&self, name: &str, dir: bool) -> bool {
        if dir {
            return true;
        }
        if self.mode == Mode::Folder {
            return false;
        }
        match self.filter.and_then(|i| self.filters.get(i)) {
            Some(f) => f.matches(name),
            None => true,
        }
    }
}

/// A shell glob over one name: `*`, `?`, `[abc]`, `[a-z]` and `[!x]`,
/// ignoring ASCII case, as file choosers do.
pub fn glob(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().map(|c| c.to_ascii_lowercase()).collect();
    let n: Vec<char> = name.chars().map(|c| c.to_ascii_lowercase()).collect();
    // Iterative with one star to back up to: linear in practice.
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() {
            match p[pi] {
                '*' => {
                    star = Some((pi, ni));
                    pi += 1;
                    continue;
                }
                '?' => {
                    pi += 1;
                    ni += 1;
                    continue;
                }
                '[' => {
                    if let Some((ok, len)) = class(&p[pi..], n[ni]) {
                        if ok {
                            pi += len;
                            ni += 1;
                            continue;
                        }
                    } else if n[ni] == '[' {
                        pi += 1;
                        ni += 1;
                        continue;
                    }
                }
                c if c == n[ni] => {
                    pi += 1;
                    ni += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star {
            Some((sp, sn)) => {
                pi = sp + 1;
                ni = sn + 1;
                star = Some((sp, sn + 1));
            }
            None => return false,
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// A `[...]` class at the start of `p`: whether `c` is in it, and its
/// length. None when the class never closes.
fn class(p: &[char], c: char) -> Option<(bool, usize)> {
    let mut i = 1;
    let negate = matches!(p.get(1), Some('!' | '^'));
    if negate {
        i += 1;
    }
    let mut hit = false;
    let mut first = true;
    while i < p.len() {
        if p[i] == ']' && !first {
            return Some((hit != negate, i + 1));
        }
        first = false;
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            hit |= p[i] <= c && c <= p[i + 2];
            i += 3;
        } else {
            hit |= p[i] == c;
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_like_a_shell_in_any_case() {
        assert!(glob("*.txt", "notes.TXT"));
        assert!(glob("*.tar.gz", "a.tar.gz"));
        assert!(!glob("*.txt", "notes.txt.bak"));
        assert!(glob("img_??.png", "IMG_01.png"));
        assert!(glob("[a-c]*", "beach.png"));
        assert!(!glob("[!a-c]*", "beach.png"));
        assert!(glob("*", ""));
        assert!(glob("a*b*c", "aXXbYYc"));
        assert!(!glob("a*b*c", "aXXbYY"));
        assert!(glob("[*", "[x"));
    }

    #[test]
    fn filters_take_globs_and_types() {
        let pictures = Filter {
            name: "Pictures".into(),
            patterns: vec![
                Pattern::Mime("image/*".into()),
                Pattern::Glob("*.dng".into()),
            ],
        };
        assert!(pictures.matches("beach.PNG"));
        assert!(pictures.matches("raw.dng"));
        assert!(!pictures.matches("plan.md"));
        let mut s = Spec::new(Mode::Open, "Media");
        s.filters.push(pictures);
        s.filter = Some(0);
        assert!(s.shows("Music", true));
        assert!(!s.shows("plan.md", false));
        s.mode = Mode::Folder;
        assert!(!s.shows("beach.png", false));
        assert_eq!(s.verb(), "Choose");
    }
}
