//! A list of rows with a selection: the shape of most settings pages.

use crate::key::Key;
use gh0st_ui::theme::Tone;
use std::cell::Cell;

/// What a key did to a [`List`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Enter on row `i`.
    Open(usize),
    /// Left (-1) or Right (+1) on row `i`: step a choice or flip a toggle.
    Change(usize, i32),
}

/// The next of `n` choices from `i`, one step in `d`'s direction, wrapping:
/// what a choice row does on Left and Right.
pub fn cycle(i: usize, d: i32, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (i as i64 + if d < 0 { -1 } else { 1 }).rem_euclid(n as i64) as usize
}

/// The selection and scroll of a list. Keep one in the page and pass it to
/// [`Ui::list`](crate::Ui::list) when drawing; it learns the row count there.
#[derive(Debug, Default)]
pub struct List {
    selected: Cell<usize>,
    first: Cell<usize>,
    len: Cell<usize>,
}

impl List {
    pub fn new() -> List {
        List::default()
    }

    pub fn selected(&self) -> usize {
        self.selected.get()
    }

    pub fn select(&mut self, i: usize) {
        self.selected.set(i);
    }

    /// Moves on Up and Down, wrapping. Returns what Enter, Left or Right
    /// asked of the selected row.
    pub fn key(&mut self, key: Key) -> Option<Action> {
        let len = self.len.get();
        if len == 0 {
            return None;
        }
        let i = self.selected.get().min(len - 1);
        match key {
            Key::Up => self.selected.set((i + len - 1) % len),
            Key::Down => self.selected.set((i + 1) % len),
            Key::Enter => return Some(Action::Open(i)),
            Key::Left => return Some(Action::Change(i, -1)),
            Key::Right => return Some(Action::Change(i, 1)),
            _ => {}
        }
        None
    }

    /// Records the row count and how many fit, and scrolls so the selection
    /// shows. Returns the first visible row.
    pub(crate) fn layout(&self, len: usize, visible: usize) -> usize {
        self.len.set(len);
        let sel = self.selected.get().min(len.saturating_sub(1));
        self.selected.set(sel);
        let mut first = self.first.get();
        if sel < first {
            first = sel;
        } else if visible > 0 && sel >= first + visible {
            first = sel + 1 - visible;
        }
        first = first.min(len.saturating_sub(visible));
        self.first.set(first);
        first
    }
}

/// The selection and scroll of a grid of tiles, as home's. Keep one in the
/// page and pass it to [`Ui::tiles`](crate::Ui::tiles); it learns the tile
/// count and the columns there.
#[derive(Debug, Default)]
pub struct Grid {
    selected: Cell<usize>,
    first_row: Cell<usize>,
    len: Cell<usize>,
    cols: Cell<usize>,
}

impl Grid {
    pub fn new() -> Grid {
        Grid::default()
    }

    pub fn selected(&self) -> usize {
        self.selected.get()
    }

    pub fn select(&mut self, i: usize) {
        self.selected.set(i);
    }

    /// Moves on the arrows: Left and Right along the row and on to the
    /// next, Up and Down a row, stopping at the ends. Returns the tile Enter
    /// opened.
    pub fn key(&mut self, key: Key) -> Option<usize> {
        let (len, cols) = (self.len.get(), self.cols.get().max(1));
        if len == 0 {
            return None;
        }
        let i = self.selected.get().min(len - 1);
        let to = match key {
            Key::Left => i.saturating_sub(1),
            Key::Right => (i + 1).min(len - 1),
            Key::Up => i.checked_sub(cols).unwrap_or(i),
            // Down from a row above a short last row lands on its end.
            Key::Down if i / cols < (len - 1) / cols => (i + cols).min(len - 1),
            Key::Enter => return Some(i),
            _ => i,
        };
        self.selected.set(to);
        None
    }

    /// Records the tile count and the columns, and scrolls by rows so the
    /// selection shows. Returns the first visible tile.
    pub(crate) fn layout(&self, len: usize, cols: usize, rows: usize) -> usize {
        self.len.set(len);
        self.cols.set(cols.max(1));
        let sel = self.selected.get().min(len.saturating_sub(1));
        self.selected.set(sel);
        let row = sel / cols.max(1);
        let mut first = self.first_row.get();
        if row < first {
            first = row;
        } else if rows > 0 && row >= first + rows {
            first = row + 1 - rows;
        }
        self.first_row.set(first);
        first * cols.max(1)
    }
}

/// Where a long text is scrolled to. Keep one in the page and pass it to
/// [`Ui::scroll_text`](crate::Ui::scroll_text); it learns the line count
/// there.
#[derive(Debug, Default)]
pub struct Scroll {
    first: Cell<usize>,
    /// Lines in all, and how many fit.
    len: Cell<usize>,
    visible: Cell<usize>,
}

impl Scroll {
    pub fn new() -> Scroll {
        Scroll::default()
    }

    /// Back to the top, for new text.
    pub fn reset(&mut self) {
        self.first.set(0);
    }

    /// Up and Down move a line. Returns false for other keys, and at either
    /// end.
    pub fn key(&mut self, key: Key) -> bool {
        let max = self.len.get().saturating_sub(self.visible.get());
        let first = self.first.get();
        let to = match key {
            Key::Up => first.saturating_sub(1),
            Key::Down => (first + 1).min(max),
            _ => return false,
        };
        self.first.set(to);
        to != first
    }

    pub(crate) fn layout(&self, len: usize, visible: usize) -> usize {
        self.len.set(len);
        self.visible.set(visible);
        let first = self.first.get().min(len.saturating_sub(visible));
        self.first.set(first);
        first
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Trail<'a> {
    None,
    Link(Option<&'a str>),
    Info(&'a str),
    Toggle(bool),
    Choice(&'a str),
    Level(u8),
    Check(bool),
    Tag(&'a str, Tone),
    Signal(u8, bool),
}

/// One row of a list.
///
/// ```
/// use gh0st_app::Row;
///
/// let rows = [
///     Row::link("Display").value("80%").icon("sun"),
///     Row::toggle("Bluetooth", false).icon("bluetooth"),
///     Row::choice("Palette", "Mint").icon("palette"),
///     Row::info("Kernel", "6.18.39"),
///     Row::info("Wi-Fi", "soon").icon("wifi").dim(),
/// ];
/// # let _ = rows;
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row<'a> {
    pub(crate) label: &'a str,
    pub(crate) icon: &'a str,
    pub(crate) trail: Trail<'a>,
    pub(crate) dim: bool,
    /// Can't be used now, and why: drawn faint, the reason on the right.
    pub(crate) off: Option<&'a str>,
    /// A second line in Caption under the label.
    pub(crate) note: Option<&'a str>,
}

impl<'a> Row<'a> {
    fn new(label: &'a str, trail: Trail<'a>) -> Row<'a> {
        Row {
            label,
            icon: "",
            trail,
            dim: false,
            off: None,
            note: None,
        }
    }

    /// A label alone.
    pub fn text(label: &'a str) -> Row<'a> {
        Row::new(label, Trail::None)
    }

    /// Opens something: a chevron on the right.
    pub fn link(label: &'a str) -> Row<'a> {
        Row::new(label, Trail::Link(None))
    }

    /// A value on the right, read only.
    pub fn info(label: &'a str, value: &'a str) -> Row<'a> {
        Row::new(label, Trail::Info(value))
    }

    /// A switch. Flip it on Enter, Left or Right.
    pub fn toggle(label: &'a str, on: bool) -> Row<'a> {
        Row::new(label, Trail::Toggle(on))
    }

    /// One of a few values, stepped with Left and Right.
    pub fn choice(label: &'a str, value: &'a str) -> Row<'a> {
        Row::new(label, Trail::Choice(value))
    }

    /// A level from 0 to 100: a bar and a percent, stepped with Left and
    /// Right.
    pub fn level(label: &'a str, percent: u8) -> Row<'a> {
        Row::new(label, Trail::Level(percent))
    }

    /// One choice of a list: a check on the one picked.
    pub fn pick(label: &'a str, picked: bool) -> Row<'a> {
        Row::new(label, Trail::Check(picked))
    }

    /// A count or a word in a tone on the right: "3", "New".
    pub fn tag(label: &'a str, word: &'a str, tone: Tone) -> Row<'a> {
        Row::new(label, Trail::Tag(word, tone))
    }

    /// A Wi-Fi network: signal bars 0 to 4, and a lock when it is secured.
    pub fn signal(label: &'a str, level: u8, secure: bool) -> Row<'a> {
        Row::new(label, Trail::Signal(level, secure))
    }

    /// Can't be used now: the row is faint and says why on its right.
    pub fn disabled(mut self, why: &'a str) -> Row<'a> {
        self.off = Some(why);
        self
    }

    /// The current value before a link's chevron.
    pub fn value(mut self, value: &'a str) -> Row<'a> {
        if let Trail::Link(_) = self.trail {
            self.trail = Trail::Link(Some(value));
        }
        self
    }

    /// A built-in icon on the left.
    /// A second line under the label, in Caption: versions, an address.
    /// A list with any such row gives every row the taller height.
    pub fn note(mut self, note: &'a str) -> Row<'a> {
        self.note = Some(note);
        self
    }

    pub fn icon(mut self, name: &'a str) -> Row<'a> {
        self.icon = name;
        self
    }

    /// A grey icon, for a row that is shown but not available yet.
    pub fn dim(mut self) -> Row<'a> {
        self.dim = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grid_moves_by_tiles_and_rows_and_stops_at_its_ends() {
        let mut g = Grid::new();
        // Seven tiles, four to a row: 0 1 2 3 / 4 5 6.
        assert_eq!(g.layout(7, 4, 2), 0);
        g.key(Key::Left);
        assert_eq!(g.selected(), 0, "the first tile stays");
        g.key(Key::Right);
        g.key(Key::Right);
        g.key(Key::Right);
        assert_eq!(g.selected(), 3);
        g.key(Key::Down);
        assert_eq!(g.selected(), 6, "below 3 is past the end: the last tile");
        g.key(Key::Down);
        assert_eq!(g.selected(), 6, "no row below");
        g.key(Key::Up);
        assert_eq!(g.selected(), 2);
        g.key(Key::Right);
        g.key(Key::Right);
        assert_eq!(g.selected(), 4, "Right runs on to the next row");
        assert_eq!(g.key(Key::Enter), Some(4));
        // One row on screen: the selected one scrolls into view.
        assert_eq!(g.layout(7, 4, 1), 4);
    }

    #[test]
    fn moves_wrap_and_actions_name_the_row() {
        let mut l = List::new();
        l.layout(3, 3);
        assert_eq!(l.key(Key::Up), None);
        assert_eq!(l.selected(), 2);
        assert_eq!(l.key(Key::Enter), Some(Action::Open(2)));
        assert_eq!(l.key(Key::Down), None);
        assert_eq!(l.key(Key::Right), Some(Action::Change(0, 1)));
    }

    #[test]
    fn a_scroll_stops_at_the_last_screen() {
        let mut s = Scroll::new();
        s.layout(5, 3);
        assert!(s.key(Key::Down));
        assert!(s.key(Key::Down));
        assert!(!s.key(Key::Down), "lines 2 to 4 show");
        assert!(s.key(Key::Up));
        assert!(!s.key(Key::Enter));
    }

    #[test]
    fn scrolls_to_keep_the_selection_visible() {
        let mut l = List::new();
        l.select(7);
        assert_eq!(l.layout(10, 4), 4);
        l.select(2);
        assert_eq!(l.layout(10, 4), 2);
    }
}
