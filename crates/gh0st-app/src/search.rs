//! Type to search a list, as home does.

use crate::field::{Edit, TextField};
use crate::key::Key;
use gh0st_core::search::score;

/// A search over a page's items. While it is active the page is typing: the
/// action bar shows the query (see [`Bars::search`](crate::Bars::search)),
/// letters type, Fn+F and Fn+X move, Enter picks and Esc leaves.
#[derive(Clone, Debug, Default)]
pub struct Search {
    field: TextField,
    /// The query in lower case, as [`score`] wants it.
    needle: String,
    active: bool,
}

impl Search {
    pub fn new() -> Search {
        Search::default()
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn query(&self) -> &str {
        self.field.text()
    }

    /// Starts with an empty query.
    pub fn start(&mut self) {
        self.active = true;
        self.field.set("");
        self.needle.clear();
    }

    /// Leaves and clears the query. Returns false if it wasn't active, so
    /// [`Page::back`](crate::Page::back) can be `!self.search.cancel()`.
    pub fn cancel(&mut self) -> bool {
        let was = self.active;
        self.active = false;
        self.field.set("");
        self.needle.clear();
        was
    }

    /// Types and deletes while active. `Submit` is Enter.
    pub fn key(&mut self, key: Key) -> Option<Edit> {
        if !self.active {
            return None;
        }
        let e = self.field.key(key)?;
        if e == Edit::Changed {
            self.needle = self.field.text().to_lowercase();
        }
        Some(e)
    }

    /// How well an item matches, lower is better, `None` for no match.
    /// `fields` go best first: a match in a later one ranks one step lower.
    /// An empty query matches everything equally.
    pub fn score(&self, fields: &[&str]) -> Option<u16> {
        if self.needle.is_empty() {
            return Some(0);
        }
        fields
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                score(&f.to_lowercase(), &self.needle).map(|s| s as u16 + i as u16)
            })
            .min()
    }

    /// The indexes of the `len` items that match, best first, ties in
    /// their own order. `score` is usually `|i| search.score(&[...])`.
    pub fn rank(&self, len: usize, score: impl Fn(usize) -> Option<u16>) -> Vec<usize> {
        let mut found: Vec<(u16, usize)> =
            (0..len).filter_map(|i| score(i).map(|s| (s, i))).collect();
        found.sort_by_key(|&(s, i)| (s, i));
        found.into_iter().map(|(_, i)| i).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_the_best_field_and_match_first() {
        let names = ["Snake", "Notes", "Tetris", "Music"];
        let about = ["a game", "write things", "blocks", "play songs"];
        let mut s = Search::new();
        s.start();
        for c in "s".chars() {
            s.key(Key::Char(c));
        }
        // A name's start, a word's start in the about line ("songs"), then
        // inside a name.
        let shown = s.rank(4, |i| s.score(&[names[i], about[i]]));
        assert_eq!(shown, [0, 3, 1, 2]);
        s.key(Key::Char('o'));
        let shown = s.rank(4, |i| s.score(&[names[i], about[i]]));
        assert_eq!(shown, [3]);
    }

    #[test]
    fn keys_do_nothing_until_started_and_cancel_clears() {
        let mut s = Search::new();
        assert_eq!(s.key(Key::Char('a')), None);
        s.start();
        s.key(Key::Char('A'));
        assert_eq!(s.query(), "A");
        assert_eq!(s.score(&["apple"]), Some(0));
        assert!(s.cancel());
        assert!(!s.cancel());
        assert_eq!(s.query(), "");
    }
}
