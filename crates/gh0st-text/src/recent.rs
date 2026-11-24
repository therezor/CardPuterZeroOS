//! Text's start page lists the text files in the recent list every app
//! shares (`gh0st_core::recent`, through `cx.recent`), newest first. What
//! Settings turns off or clears there is gone here too.

use std::path::{Path, PathBuf};

/// Paths shown.
pub const MAX: usize = 20;

#[derive(Default)]
pub struct Recent {
    list: Vec<PathBuf>,
}

/// A file the editor opens as text, not as hex.
fn is_text(p: &Path) -> bool {
    gh0st_core::mime::of_path(p).is_ok_and(|m| gh0st_core::mime::is_a(m, "text/plain"))
}

impl Recent {
    /// The text files among `shared`, newest first.
    pub fn from(shared: Vec<PathBuf>) -> Recent {
        Recent {
            list: shared
                .into_iter()
                .filter(|p| is_text(p))
                .take(MAX)
                .collect(),
        }
    }

    pub fn list(&self) -> &[PathBuf] {
        &self.list
    }

    /// `p` goes first, for this run; `cx.recent` keeps it for the next.
    pub fn add(&mut self, p: &Path) {
        if !p.is_absolute() {
            return;
        }
        self.list.retain(|q| q != p);
        self.list.insert(0, p.to_path_buf());
        self.list.truncate(MAX);
    }

    pub fn remove(&mut self, p: &Path) {
        self.list.retain(|q| q != p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_files_only_newest_first_once_each() {
        let d = std::env::temp_dir().join(format!("gh0st-text-recent-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (a, b, pic) = (d.join("a.txt"), d.join("b.rs"), d.join("p.png"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"fn main() {}").unwrap();
        std::fs::write(&pic, b"\x89PNG\r\n\x1a\n").unwrap();
        let mut r = Recent::from(vec![b.clone(), pic, a.clone()]);
        assert_eq!(r.list(), [b.clone(), a.clone()]);
        r.add(&a);
        r.add(Path::new("relative"));
        assert_eq!(r.list(), [a, b]);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
