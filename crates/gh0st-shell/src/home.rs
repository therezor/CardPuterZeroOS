//! The home screen: every app as a grid of tiles or a list of rows, the
//! user's pick (`gh0st.home_layout`). Nav mode moves and opens. Search mode,
//! entered with S, filters as you type.
//!
//! Motion is two short things (`gh0st.animations` turns them off): the
//! selection frame glides to the new item, and a page change slides the grid
//! or scrolls the list. Each runs a fixed time with an ease-out, and the shell
//! draws frames only while one runs.

use crate::motion::{self, lerp, lerp_rect};
use crate::search::score;
use gh0st_ui::Canvas;
use gh0st_ui::geom::Rect;
use gh0st_ui::image::Image;
use gh0st_ui::theme::Theme;
use gh0st_ui::widgets::{self, AppItem, Status, Visual};
use std::rc::Rc;

pub use gh0st_core::apps::Layout;

/// Home's chips: 5 Find, 6 Apps, and the menu on 8, lit while it is open.
pub fn chips(menu_open: bool) -> [widgets::Chip<'static>; 5] {
    use widgets::{Chip, Look};
    let none = Chip::default();
    let chip = |icon, word| Chip {
        icon,
        word,
        look: Look::Normal,
    };
    let menu = Chip {
        icon: "menu",
        word: "",
        look: if menu_open { Look::On } else { Look::Normal },
    };
    // Under the menu its own chip alone shows, lit, as in every app.
    if menu_open {
        return [none, none, none, none, menu];
    }
    [none, chip("search", "Find"), chip("arrow-left-right", "Apps"), none, menu]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Nav,
    Search,
}

#[derive(Clone)]
pub enum Icon {
    /// A system app's symbol (`Icon=gh0st:`), in the theme's own colours.
    System { name: &'static str },
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
    /// For a top-bar slot.
    pub tiny: Image,
}

impl Icon {
    /// Loads a PNG for both badge sizes, leaving an inset inside each, and
    /// for a bar slot.
    pub fn picture(path: &std::path::Path, t: &Theme) -> Result<Icon, String> {
        let large = Image::load_png(path, (t.m.badge - 6) as u32)?;
        let small = Image::load_png(path, (t.m.badge_small - 4) as u32)?;
        let tiny = Image::load_png(path, t.m.slot_icon as u32)?;
        Ok(Icon::Picture(Rc::new(Picture { large, small, tiny })))
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

const GLIDE_MS: u64 = 80;
const SLIDE_MS: u64 = 180;

/// Where each item sits, worked out once from the theme.
#[derive(Clone, Copy, Debug)]
struct Geo {
    cols: usize,
    rows: usize,
    grid: Rect,
    tile_w: i32,
    tile_h: i32,
    gap: i32,
    list: Rect,
    list_rows: usize,
    row_h: i32,
}

impl Geo {
    fn new(t: &Theme) -> Geo {
        let area = widgets::grid(t);
        let (cols, rows) = (t.m.cols, t.m.rows);
        let tile_w = (area.w - (cols - 1) * t.m.gap) / cols;
        let tile_h = (area.h - (rows - 1) * t.m.gap) / rows;
        // Spread the rounding left over by the division into the margin.
        let x0 = area.x + (area.w - (cols * tile_w + (cols - 1) * t.m.gap)) / 2;
        let (list, list_rows) = widgets::rows_area(t, t.m.app_row_h);
        Geo {
            cols: cols as usize,
            rows: rows as usize,
            grid: Rect::new(x0, area.y, area.w - 2 * (x0 - area.x), area.h),
            tile_w,
            tile_h,
            gap: t.m.gap,
            list: Rect::new(list.x, list.y, list.w - 4, list.h),
            list_rows,
            row_h: t.m.app_row_h,
        }
    }

    fn tile(&self, slot: usize) -> Rect {
        let (col, row) = ((slot % self.cols) as i32, (slot / self.cols) as i32);
        Rect::new(
            self.grid.x + col * (self.tile_w + self.gap),
            self.grid.y + row * (self.tile_h + self.gap),
            self.tile_w,
            self.tile_h,
        )
    }

    fn row(&self, slot: i32) -> Rect {
        Rect::new(
            self.list.x,
            self.list.y + slot * self.row_h,
            self.list.w,
            self.row_h,
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum Motion {
    /// The selection frame moves from `from` to the selected item.
    Glide { from: Rect },
    /// The grid changes page. `dir` is +1 for the next page.
    Page { from_top: usize, dir: i32 },
    /// The list scrolls. Rows start `from_px` lower than where they end,
    /// and the frame glides from `from`.
    Scroll { from_px: i32, from: Rect },
}

type Anim = motion::Anim<Motion>;

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
    geo: Geo,
    animations: bool,
    anim: Option<Anim>,
}

impl Home {
    pub fn new(entries: Vec<Entry>, layout: Layout, t: &Theme) -> Home {
        let shown = (0..entries.len()).collect();
        Home {
            layout,
            mode: Mode::Nav,
            query: String::new(),
            entries,
            shown,
            selected: 0,
            top: 0,
            geo: Geo::new(t),
            animations: true,
            anim: None,
        }
    }

    pub fn set_animations(&mut self, on: bool) {
        self.animations = on;
        if !on {
            self.anim = None;
        }
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn set_layout(&mut self, layout: Layout) {
        if layout != self.layout {
            self.anim = None;
        }
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
            Layout::Grid => self.geo.cols * self.geo.rows,
            Layout::List => self.geo.list_rows,
        }
    }

    /// The selection frame's place for item `index` with the screen starting
    /// at `top`.
    fn frame(&self, index: usize, top: usize) -> Rect {
        match self.layout {
            Layout::Grid => self.geo.tile(index.saturating_sub(top)),
            Layout::List => self.geo.row(index as i32 - top as i32),
        }
    }

    /// Arrow movement. In the grid, left and right step and wrap into the next
    /// row and page; up and down move a row. In the list, up and down step and
    /// left and right jump a screen.
    pub fn move_by(&mut self, dx: i32, dy: i32, now: u64) {
        let n = self.shown.len();
        if n == 0 {
            return;
        }
        let (old, old_top) = (self.selected, self.top);
        let from = self.current_frame(now);
        let last = n - 1;
        let s = self.selected as i64;
        let target = match self.layout {
            Layout::Grid => {
                let cols = self.geo.cols as i64;
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
            Layout::List => s + dy as i64 + dx as i64 * self.geo.list_rows as i64,
        };
        self.selected = target.clamp(0, last as i64) as usize;
        self.keep_visible();
        if !self.animations || self.selected == old {
            return;
        }
        let from = from.unwrap_or_else(|| self.frame(old, old_top));
        let motion = match self.layout {
            _ if self.top == old_top => Motion::Glide { from },
            Layout::Grid => Motion::Page {
                from_top: old_top,
                dir: if self.top > old_top { 1 } else { -1 },
            },
            Layout::List => Motion::Scroll {
                from_px: (self.top as i32 - old_top as i32) * self.geo.row_h,
                from: Rect::new(from.x, from.y, from.w, from.h),
            },
        };
        let ms = match motion {
            Motion::Page { .. } => SLIDE_MS,
            _ => GLIDE_MS,
        };
        self.anim = Some(Anim {
            start: now,
            ms,
            motion,
        });
    }

    /// Where the frame is drawn right now, mid-glide included, so a quick
    /// second press starts from what the eye sees.
    fn current_frame(&self, now: u64) -> Option<Rect> {
        let a = self.anim?;
        let to = self.frame(self.selected, self.top);
        match a.motion {
            Motion::Glide { from } | Motion::Scroll { from, .. } => {
                Some(lerp_rect(from, to, a.progress(now)))
            }
            Motion::Page { .. } => None,
        }
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
        self.anim = None;
    }

    /// When the next animation frame is due, if anything moves.
    pub fn next_frame(&self, now: u64) -> Option<u64> {
        self.anim.map(|a| a.next_frame(now))
    }

    /// Drops a finished animation. Called after each drawn frame.
    pub fn settle(&mut self, now: u64) {
        if self.anim.is_some_and(|a| a.done(now)) {
            self.anim = None;
        }
    }

    pub fn draw(&self, c: &mut Canvas, t: &Theme, status: &Status, now: u64) {
        c.fill(widgets::body(t), t.c.bg);
        widgets::status_bar(c, t, status);
        if self.shown.is_empty() {
            let msg = format!("No app matches \u{201c}{}\u{201d}", self.query);
            widgets::text_centre(c, t.f.body, widgets::content(t), &msg, t.c.muted);
        } else {
            let old = c.set_clip(widgets::body(t));
            match self.layout {
                Layout::Grid => self.draw_grid(c, t, now),
                Layout::List => self.draw_list(c, t, now),
            }
            c.set_clip(old);
        }
        match self.mode {
            Mode::Search => widgets::search_bar(c, t, &self.query),
            Mode::Nav => {
                widgets::action_bar(c, t, &chips(false));
                if let Layout::Grid = self.layout {
                    let per = self.per_screen();
                    let (page, count) = (self.top / per, self.shown.len().div_ceil(per));
                    let cy = t.m.height - t.m.hint_h / 2;
                    widgets::page_dots(c, t, t.m.width - t.m.pad, cy, page, count);
                }
            }
        }
    }

    fn item<'a>(&'a self, t: &Theme, index: usize, small: bool) -> AppItem<'a> {
        let e = &self.entries[self.shown[index]];
        AppItem {
            label: &e.name,
            note: &e.note,
            visual: visual(
                t,
                &e.icon,
                &e.name,
                if small { Size::Small } else { Size::Large },
            ),
            pinned: e.pinned,
        }
    }

    /// Draws one grid page starting at `top`, shifted `dx` pixels.
    fn grid_page(&self, c: &mut Canvas, t: &Theme, top: usize, dx: i32, frame: Option<Rect>) {
        let frame = frame.map(|f| Rect::new(f.x + dx, f.y, f.w, f.h));
        if let Some(f) = frame {
            widgets::selection(c, t, f);
        }
        let per = self.per_screen();
        for index in top..(top + per).min(self.shown.len()) {
            let r = self.geo.tile(index - top);
            let r = Rect::new(r.x + dx, r.y, r.w, r.h);
            let item = self.item(t, index, false);
            widgets::tile_content(c, t, r, &item, index == self.selected);
        }
        if let Some(f) = frame {
            widgets::selection_ring(c, t, f);
        }
    }

    fn draw_grid(&self, c: &mut Canvas, t: &Theme, now: u64) {
        let to = self.frame(self.selected, self.top);
        match self.anim {
            Some(
                a @ Anim {
                    motion: Motion::Page { from_top, dir },
                    ..
                },
            ) => {
                let p = a.progress(now);
                let dist = self.geo.grid.w + self.geo.gap;
                let shift = (dist as f32 * p).round() as i32;
                self.grid_page(c, t, from_top, -dir * shift, None);
                self.grid_page(c, t, self.top, dir * (dist - shift), Some(to));
            }
            Some(
                a @ Anim {
                    motion: Motion::Glide { from },
                    ..
                },
            ) => {
                let f = lerp_rect(from, to, a.progress(now));
                self.grid_page(c, t, self.top, 0, Some(f));
            }
            _ => self.grid_page(c, t, self.top, 0, Some(to)),
        }
    }

    fn draw_list(&self, c: &mut Canvas, t: &Theme, now: u64) {
        let to = self.frame(self.selected, self.top);
        let (frame, offset) = match self.anim {
            Some(
                a @ Anim {
                    motion: Motion::Glide { from },
                    ..
                },
            ) => (lerp_rect(from, to, a.progress(now)), 0),
            Some(
                a @ Anim {
                    motion: Motion::Scroll { from_px, from },
                    ..
                },
            ) => {
                let p = a.progress(now);
                (lerp_rect(from, to, p), lerp(from_px, 0, p))
            }
            _ => (to, 0),
        };
        let old = c.set_clip(self.geo.list.intersect(&c.clip()));
        widgets::selection(c, t, frame);
        // Rows from one above to one below the screen, so a scroll never
        // shows a gap at either edge.
        let rows = self.geo.list_rows as i32;
        for slot in -1..=rows {
            let index = self.top as i32 + slot;
            if index < 0 || index as usize >= self.shown.len() {
                continue;
            }
            let r = self.geo.row(slot);
            let r = Rect::new(r.x, r.y + offset, r.w, r.h);
            let item = self.item(t, index as usize, true);
            widgets::app_row_content(c, t, r, &item, index as usize == self.selected);
        }
        widgets::selection_ring(c, t, frame);
        c.set_clip(old);
        let track = self.geo.list;
        widgets::scrollbar(c, t, track, self.top, self.geo.list_rows, self.shown.len());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// A grid tile or switcher card.
    Large,
    /// A list row.
    Small,
    /// A top-bar slot.
    Tiny,
}

/// How an app's icon draws at `size`.
pub fn visual<'a>(t: &Theme, icon: &'a Icon, name: &str, size: Size) -> Visual<'a> {
    let tint = |i: usize| t.c.tints[i % t.c.tints.len()];
    match icon {
        Icon::System { name } => Visual::System { icon: name },
        Icon::Symbol { name, tint: i } => Visual::Badge {
            icon: name,
            tint: tint(*i),
        },
        Icon::Picture(p) => Visual::Image(match size {
            Size::Large => &p.large,
            Size::Small => &p.small,
            Size::Tiny => &p.tiny,
        }),
        Icon::Letter { tint: i } => Visual::Letter {
            ch: name.chars().next().unwrap_or('?'),
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
        h.move_by(1, 0, 0);
        assert_eq!((h.selected, h.top), (8, 8));
    }

    #[test]
    fn grid_down_into_a_short_row_lands_on_its_last_tile() {
        let mut h = home(&TEN, Layout::Grid);
        h.select(7);
        h.move_by(0, 1, 0);
        assert_eq!(h.selected, 9, "row three has only two tiles");
        h.move_by(0, 1, 0);
        assert_eq!(h.selected, 9, "nothing below");
        h.move_by(0, -1, 0);
        assert_eq!(h.selected, 5);
    }

    #[test]
    fn list_scrolls_only_when_the_selection_leaves_the_screen() {
        let mut h = home(&TEN, Layout::List);
        let rows = h.per_screen();
        assert_eq!(rows, 4);
        h.move_by(0, 3, 0);
        assert_eq!(h.top, 0);
        h.move_by(0, 1, 0);
        assert_eq!((h.selected, h.top), (4, 1));
        h.move_by(-1, 0, 0);
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
    fn a_move_starts_a_glide_and_a_page_turn_starts_a_slide() {
        let mut h = home(&TEN, Layout::Grid);
        h.move_by(1, 0, 1000);
        assert!(matches!(h.anim.unwrap().motion, Motion::Glide { .. }));
        assert_eq!(h.next_frame(1000), Some(1000 + motion::FRAME_MS));
        h.settle(1000 + GLIDE_MS);
        assert!(h.anim.is_none());
        h.select(7);
        h.move_by(1, 0, 2000);
        assert!(matches!(
            h.anim.unwrap().motion,
            Motion::Page { dir: 1, .. }
        ));
    }

    #[test]
    fn the_last_frame_lands_exactly_and_nothing_moves_with_animations_off() {
        let mut h = home(&TEN, Layout::List);
        h.move_by(0, 1, 0);
        assert_eq!(h.next_frame(GLIDE_MS - 5), Some(GLIDE_MS));
        let a = h.anim.unwrap();
        assert_eq!(a.progress(GLIDE_MS), 1.0);
        h.set_animations(false);
        h.move_by(0, 1, 0);
        assert!(h.anim.is_none());
        assert_eq!(h.next_frame(0), None);
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
