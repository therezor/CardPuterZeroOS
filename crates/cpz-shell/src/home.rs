//! The home screen: every app as a grid of tiles or a list of rows, the
//! user's pick (`cpz.home_layout`). Nav mode moves and opens. Search mode,
//! entered with S, filters as you type.

use crate::search::score;
use cpz_ui::Canvas;
use cpz_ui::geom::Rect;
use cpz_ui::image::Image;
use cpz_ui::theme::Theme;
use cpz_ui::widgets::{self, AppRow, Hint, Status, Visual};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Grid,
    List,
}

impl Layout {
    pub fn from_config(s: &str) -> Option<Layout> {
        match s {
            "grid" => Some(Layout::Grid),
            "list" => Some(Layout::List),
            _ => None,
        }
    }

    pub fn config_name(self) -> &'static str {
        match self {
            Layout::Grid => "grid",
            Layout::List => "list",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Layout::Grid => "Grid",
            Layout::List => "List",
        }
    }

    pub fn toggled(self) -> Layout {
        match self {
            Layout::Grid => Layout::List,
            Layout::List => Layout::Grid,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Nav,
    Search,
}

pub enum Icon {
    /// A baked symbol on a badge in theme tint `tint`.
    Symbol { name: &'static str, tint: usize },
    /// The app's own PNG, scaled once for each badge size. Shared, so a
    /// rescan reuses it instead of decoding again.
    Picture(Rc<Picture>),
    /// No usable icon: the first letter of the name.
    Letter { tint: usize },
}

pub struct Picture {
    pub large: Image,
    pub small: Image,
}

impl Icon {
    /// Loads a PNG for both badge sizes, leaving a 3 px inset inside each.
    pub fn picture(path: &std::path::Path, t: &Theme) -> Result<Icon, String> {
        let large = Image::load_png(path, (t.m.badge - 6) as u32)?;
        let small = Image::load_png(path, (t.m.badge_small - 4) as u32)?;
        Ok(Icon::Picture(Rc::new(Picture { large, small })))
    }
}

pub struct Entry {
    pub id: String,
    pub name: String,
    /// The second line in the list layout (`Comment=`). May be empty.
    pub note: String,
    pub icon: Icon,
    pub pinned: bool,
}

/// How many items one screen holds in each layout, from the theme.
#[derive(Clone, Copy, Debug)]
struct Fit {
    cols: usize,
    rows: usize,
    list_rows: usize,
}

pub struct Home {
    layout: Layout,
    mode: Mode,
    query: String,
    entries: Vec<Entry>,
    /// Indices into `entries`, in the order shown.
    shown: Vec<usize>,
    /// Index into `shown`.
    selected: usize,
    /// First item of `shown` on screen.
    top: usize,
    fit: Fit,
}

impl Home {
    pub fn new(entries: Vec<Entry>, layout: Layout, t: &Theme) -> Home {
        let fit = Fit {
            cols: t.m.cols as usize,
            rows: t.m.rows as usize,
            list_rows: widgets::rows_area(t, t.m.app_row_h).1,
        };
        let shown = (0..entries.len()).collect();
        Home {
            layout,
            mode: Mode::Nav,
            query: String::new(),
            entries,
            shown,
            selected: 0,
            top: 0,
            fit,
        }
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
        self.keep_visible();
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn selected(&self) -> Option<&Entry> {
        self.shown.get(self.selected).map(|&i| &self.entries[i])
    }

    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.shown.len().saturating_sub(1));
        self.keep_visible();
    }

    /// The `n`th item on screen (0-based), for the digit keys.
    pub fn select_on_screen(&mut self, n: usize) -> bool {
        if n >= self.per_screen() || self.top + n >= self.shown.len() {
            return false;
        }
        self.selected = self.top + n;
        true
    }

    /// New app list after a rescan, keeping the selection on the same app.
    pub fn replace_entries(&mut self, entries: Vec<Entry>) {
        let keep = self.selected().map(|e| e.id.clone());
        self.entries = entries;
        self.refilter();
        if let Some(id) = keep {
            if let Some(pos) = self.shown.iter().position(|&i| self.entries[i].id == id) {
                self.select(pos);
            }
        }
    }

    pub fn start_search(&mut self) {
        self.mode = Mode::Search;
    }

    pub fn type_char(&mut self, ch: char) {
        self.query.push(ch);
        self.refilter();
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.refilter();
    }

    /// Esc in search: back to nav mode with every app shown.
    pub fn cancel_search(&mut self) {
        self.mode = Mode::Nav;
        self.query.clear();
        self.refilter();
    }

    fn per_screen(&self) -> usize {
        match self.layout {
            Layout::Grid => self.fit.cols * self.fit.rows,
            Layout::List => self.fit.list_rows,
        }
    }

    /// Arrow movement. In the grid, left and right step and wrap into the next
    /// row and page; up and down move a row. In the list, up and down step and
    /// left and right jump a screen.
    pub fn move_by(&mut self, dx: i32, dy: i32) {
        let n = self.shown.len();
        if n == 0 {
            return;
        }
        let last = n - 1;
        let s = self.selected as i64;
        let target = match self.layout {
            Layout::Grid => {
                let cols = self.fit.cols as i64;
                let mut t = s + dx as i64;
                if dy > 0 {
                    let next_row_start = (s / cols + 1) * cols;
                    t = if s + cols <= last as i64 {
                        s + cols
                    } else if next_row_start <= last as i64 {
                        last as i64
                    } else {
                        s
                    };
                } else if dy < 0 {
                    t = if s >= cols { s - cols } else { s };
                }
                t
            }
            Layout::List => s + dy as i64 + dx as i64 * self.fit.list_rows as i64,
        };
        self.selected = target.clamp(0, last as i64) as usize;
        self.keep_visible();
    }

    fn keep_visible(&mut self) {
        let per = self.per_screen();
        match self.layout {
            Layout::Grid => self.top = self.selected / per * per,
            Layout::List => {
                if self.selected < self.top {
                    self.top = self.selected;
                } else if self.selected >= self.top + per {
                    self.top = self.selected + 1 - per;
                }
                // Never leave empty rows at the bottom while items sit above.
                self.top = self.top.min(self.shown.len().saturating_sub(per));
            }
        }
    }

    fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        let mut ranked: Vec<(u8, usize)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let by_name = score(&e.name.to_lowercase(), &q);
                let by_id = score(&e.id.to_lowercase(), &q).map(|s| s + 1);
                by_name.into_iter().chain(by_id).min().map(|s| (s, i))
            })
            .collect();
        ranked.sort_by_key(|&(s, i)| (s, i));
        self.shown = ranked.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
        self.top = 0;
    }

    pub fn draw(&self, c: &mut Canvas, t: &Theme, status: &Status) {
        c.fill(widgets::body(t), t.c.bg);
        widgets::status_bar(c, t, status);
        let area = widgets::content(t);
        if self.shown.is_empty() {
            let msg = format!("No app matches \u{201c}{}\u{201d}", self.query);
            widgets::text_centre(c, t.f.body, area, &msg, t.c.muted);
        } else {
            match self.layout {
                Layout::Grid => self.draw_grid(c, t, area),
                Layout::List => self.draw_list(c, t),
            }
        }
        match self.mode {
            Mode::Search => widgets::search_bar(c, t, &self.query),
            Mode::Nav => {
                let hints = [
                    Hint {
                        key: "S",
                        label: "search",
                    },
                    Hint {
                        key: "P",
                        label: "pin",
                    },
                    Hint {
                        key: "Enter",
                        label: "open",
                    },
                ];
                let pages = match self.layout {
                    Layout::Grid => {
                        let per = self.per_screen();
                        Some((self.top / per, self.shown.len().div_ceil(per)))
                    }
                    Layout::List => None,
                };
                widgets::hint_bar(c, t, &hints, pages);
            }
        }
    }

    fn draw_grid(&self, c: &mut Canvas, t: &Theme, area: Rect) {
        let (cols, rows) = (t.m.cols, t.m.rows);
        let w = (area.w - (cols - 1) * t.m.gap) / cols;
        let h = (area.h - (rows - 1) * t.m.gap) / rows;
        // Spread the rounding left over by the division into the outer margin.
        let x0 = area.x + (area.w - (cols * w + (cols - 1) * t.m.gap)) / 2;
        for (slot, &i) in self
            .shown
            .iter()
            .skip(self.top)
            .take(self.per_screen())
            .enumerate()
        {
            let (col, row) = (slot as i32 % cols, slot as i32 / cols);
            let r = Rect::new(x0 + col * (w + t.m.gap), area.y + row * (h + t.m.gap), w, h);
            let e = &self.entries[i];
            let selected = self.top + slot == self.selected;
            widgets::tile(c, t, r, &e.name, &visual(t, e, false), selected);
        }
    }

    fn draw_list(&self, c: &mut Canvas, t: &Theme) {
        let h = t.m.app_row_h;
        let (area, rows) = widgets::rows_area(t, h);
        for (slot, &i) in self.shown.iter().skip(self.top).take(rows).enumerate() {
            let r = Rect::new(area.x, area.y + slot as i32 * h, area.w - 4, h);
            let e = &self.entries[i];
            let row = AppRow {
                label: &e.name,
                note: &e.note,
                visual: visual(t, e, true),
                pinned: e.pinned,
            };
            widgets::app_row(c, t, r, &row, self.top + slot == self.selected);
        }
        let track = Rect::new(area.x, area.y, area.w - 4, area.h);
        widgets::scrollbar(c, t, track, self.top, rows, self.shown.len());
    }
}

fn visual<'a>(t: &Theme, e: &'a Entry, small: bool) -> Visual<'a> {
    let tint = |i: usize| t.c.tints[i % t.c.tints.len()];
    match &e.icon {
        Icon::Symbol { name, tint: i } => Visual::Badge {
            icon: name,
            tint: tint(*i),
        },
        Icon::Picture(p) => Visual::Image(if small { &p.small } else { &p.large }),
        Icon::Letter { tint: i } => Visual::Letter {
            ch: e.name.chars().next().unwrap_or('?'),
            tint: tint(*i),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(names: &[&str], layout: Layout) -> Home {
        let entries = names
            .iter()
            .map(|n| Entry {
                id: n.to_lowercase(),
                name: n.to_string(),
                note: String::new(),
                icon: Icon::Letter { tint: 0 },
                pinned: false,
            })
            .collect();
        Home::new(entries, layout, &Theme::default())
    }

    const TEN: [&str; 10] = [
        "Terminal",
        "Settings",
        "Store",
        "SSH",
        "Camera",
        "Files",
        "Music",
        "Recorder",
        "Notes",
        "Keyboard Guide",
    ];

    #[test]
    fn grid_right_from_the_last_tile_of_a_page_turns_the_page() {
        let mut h = home(&TEN, Layout::Grid);
        h.select(7);
        assert_eq!(h.top, 0);
        h.move_by(1, 0);
        assert_eq!((h.selected, h.top), (8, 8));
    }

    #[test]
    fn grid_down_into_a_short_row_lands_on_its_last_tile() {
        let mut h = home(&TEN, Layout::Grid);
        h.select(7);
        h.move_by(0, 1);
        assert_eq!(h.selected, 9, "row three has only two tiles");
        h.move_by(0, 1);
        assert_eq!(h.selected, 9, "nothing below");
        h.move_by(0, -1);
        assert_eq!(h.selected, 5);
    }

    #[test]
    fn list_scrolls_only_when_the_selection_leaves_the_screen() {
        let mut h = home(&TEN, Layout::List);
        let rows = h.per_screen();
        assert_eq!(rows, 4);
        h.move_by(0, 3);
        assert_eq!(h.top, 0);
        h.move_by(0, 1);
        assert_eq!((h.selected, h.top), (4, 1));
        h.move_by(-1, 0);
        assert_eq!((h.selected, h.top), (0, 0));
    }

    #[test]
    fn search_ranks_a_prefix_first_and_esc_brings_everything_back() {
        let mut h = home(&TEN, Layout::Grid);
        h.start_search();
        h.type_char('r');
        h.type_char('e');
        let names: Vec<&str> = h
            .shown
            .iter()
            .map(|&i| h.entries[i].name.as_str())
            .collect();
        assert_eq!(names[0], "Recorder");
        assert!(names.contains(&"Store"));
        assert!(!names.contains(&"Camera"));
        h.cancel_search();
        assert_eq!(h.mode(), Mode::Nav);
        assert_eq!(h.shown.len(), TEN.len());
    }

    #[test]
    fn switching_layout_keeps_the_selection_on_screen() {
        let mut h = home(&TEN, Layout::List);
        h.select(9);
        h.set_layout(Layout::Grid);
        assert_eq!(h.top, 8);
        h.set_layout(Layout::List);
        assert!(h.top <= 9 && 9 < h.top + 4);
    }
}
