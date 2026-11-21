//! What part of the terminal the screen shows: the face and zoom, fit or
//! window, where the window is panned to, and how far back in history.

use crate::vt::Terminal;
use crate::vt::history::History;
use gh0st_app::gfx::theme::{self, TermFace};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Terminus,
    Fixed,
}

impl Family {
    pub fn faces(self) -> &'static [TermFace] {
        match self {
            Family::Terminus => theme::TERM_TERMINUS,
            Family::Fixed => theme::TERM_FIXED,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Family::Terminus => "terminus",
            Family::Fixed => "fixed",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Family::Terminus => "Terminus",
            Family::Fixed => "Fixed",
        }
    }

    pub fn from_key(s: &str) -> Family {
        // "jetbrains" was the anti-aliased face, now gone: Terminus.
        if s == "fixed" {
            Family::Fixed
        } else {
            Family::Terminus
        }
    }
}

/// The first row on screen: a grid row, or a row of a history line (by its
/// stable id and which of its wrapped rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pos {
    Grid(usize),
    Hist(u64, usize),
}

impl Pos {
    /// History first, oldest line first, then the grid from the top.
    pub fn key(self) -> (u8, u64, usize) {
        match self {
            Pos::Hist(id, sub) => (0, id, sub),
            Pos::Grid(y) => (1, 0, y),
        }
    }

    pub fn up(self, t: &Terminal) -> Pos {
        step_up(self, &t.history, t.cols)
    }

    pub fn down(self, t: &Terminal) -> Pos {
        step_down(self, &t.history, t.cols, t.rows - 1)
    }

    pub fn src(self, t: &Terminal) -> Src {
        let h = &t.history;
        match clamp(self, h) {
            Pos::Grid(y) => Src::Grid(y),
            Pos::Hist(id, sub) => Src::Hist((id - h.id(0)) as usize, sub),
        }
    }
}

/// One row to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Src {
    Grid(usize),
    /// History line index and its wrapped row.
    Hist(usize, usize),
}

pub struct View {
    pub family: Family,
    pub zoom: usize,
    /// Window mode: the grid is this size and the screen pans over it.
    /// None: fit mode, the grid is the screen.
    pub window: Option<(usize, usize)>,
    /// First column on screen, in window mode.
    pub pan_x: usize,
    pub top: Pos,
    /// Keep the cursor in sight. Off once the user pans or scrolls.
    pub follow: bool,
}

/// Cells kept between the cursor and the window's edge.
const MARGIN: usize = 2;

impl View {
    pub fn new(family: Family, zoom: usize, window: Option<(usize, usize)>) -> View {
        let zoom = zoom.min(family.faces().len() - 1);
        View {
            family,
            zoom,
            window,
            pan_x: 0,
            top: Pos::Grid(0),
            follow: true,
        }
    }

    pub fn face(&self) -> &'static TermFace {
        &self.family.faces()[self.zoom]
    }

    pub fn live(&self) -> bool {
        matches!(self.top, Pos::Grid(_))
    }

    /// Back to the cursor, as typing does.
    pub fn snap(&mut self) {
        self.follow = true;
        self.top = Pos::Grid(0);
    }

    /// Keeps the cursor in the window. `vis` is the cells on screen.
    pub fn follow_cursor(
        &mut self,
        cursor: (usize, usize),
        grid: (usize, usize),
        vis: (usize, usize),
    ) {
        if !self.follow {
            return;
        }
        let fit = |pan: usize, at: usize, n: usize, size: usize| -> usize {
            let m = MARGIN.min(n.saturating_sub(1) / 2);
            let mut p = pan;
            if at < p + m {
                p = at.saturating_sub(m);
            }
            if at + m >= p + n {
                p = at + m + 1 - n;
            }
            p.min(size.saturating_sub(n))
        };
        self.pan_x = fit(self.pan_x, cursor.0, vis.0, grid.0);
        let y = match self.top {
            Pos::Grid(y) => y,
            Pos::Hist(..) => 0,
        };
        self.top = Pos::Grid(fit(y, cursor.1, vis.1, grid.1));
    }

    pub fn pan(&mut self, dx: isize, grid_cols: usize, vis_cols: usize) {
        let max = grid_cols.saturating_sub(vis_cols) as isize;
        self.pan_x = (self.pan_x as isize + dx).clamp(0, max) as usize;
        self.follow = false;
    }

    /// Moves the view `n` rows up (into history) or down.
    pub fn scroll(&mut self, up: bool, n: usize, t: &Terminal, vis_rows: usize) {
        let h = &t.history;
        let max_y = t.rows.saturating_sub(vis_rows);
        for _ in 0..n {
            self.top = if up {
                step_up(self.top, h, t.cols)
            } else {
                step_down(self.top, h, t.cols, max_y)
            };
        }
        self.follow = match self.top {
            Pos::Grid(y) => self.window.is_none() || y == max_y,
            Pos::Hist(..) => false,
        };
    }

    /// The rows on screen, top first.
    pub fn rows(&self, t: &Terminal, vis_rows: usize) -> Vec<Src> {
        let h = &t.history;
        let mut out = Vec::with_capacity(vis_rows);
        let mut pos = clamp(self.top, h);
        for _ in 0..vis_rows {
            out.push(match pos {
                Pos::Grid(y) => Src::Grid(y),
                Pos::Hist(id, sub) => Src::Hist((id - h.id(0)) as usize, sub),
            });
            pos = step_down(pos, h, t.cols, usize::MAX);
        }
        out
    }

    /// Scrolls so row `p` is on screen.
    pub fn reveal(&mut self, p: Pos, t: &Terminal, vis_rows: usize) {
        if p.key() < clamp(self.top, &t.history).key() {
            self.top = p;
            self.follow = false;
            return;
        }
        let mut bottom = self.top;
        for _ in 1..vis_rows {
            bottom = step_down(bottom, &t.history, t.cols, usize::MAX);
        }
        if p.key() > bottom.key() {
            let mut top = p;
            for _ in 1..vis_rows {
                top = step_up(top, &t.history, t.cols);
            }
            let max_y = t.rows.saturating_sub(vis_rows);
            self.top = match top {
                Pos::Grid(y) => Pos::Grid(y.min(max_y)),
                h => h,
            };
            self.follow = false;
        }
    }

    /// How far back the view is, from 0 (live) to 1 (oldest line), for the
    /// scroll mark.
    pub fn depth(&self, t: &Terminal) -> Option<f32> {
        let h = &t.history;
        match clamp(self.top, h) {
            Pos::Grid(_) => None,
            Pos::Hist(id, _) => {
                let n = h.len().max(1) as f32;
                Some(1.0 - (id - h.id(0)) as f32 / n)
            }
        }
    }
}

/// History may have dropped the line a position points at.
fn clamp(p: Pos, h: &History) -> Pos {
    match p {
        Pos::Hist(_, _) if h.is_empty() => Pos::Grid(0),
        Pos::Hist(id, _) if id < h.id(0) => Pos::Hist(h.id(0), 0),
        p => p,
    }
}

fn step_up(p: Pos, h: &History, cols: usize) -> Pos {
    match clamp(p, h) {
        Pos::Grid(y) if y > 0 => Pos::Grid(y - 1),
        Pos::Grid(_) if h.is_empty() => Pos::Grid(0),
        Pos::Grid(_) => {
            let last = h.len() - 1;
            Pos::Hist(h.id(last), h.line(last).rows(cols) - 1)
        }
        Pos::Hist(id, sub) if sub > 0 => Pos::Hist(id, sub - 1),
        Pos::Hist(id, _) if id > h.id(0) => {
            let i = (id - 1 - h.id(0)) as usize;
            Pos::Hist(id - 1, h.line(i).rows(cols) - 1)
        }
        p => p,
    }
}

fn step_down(p: Pos, h: &History, cols: usize, max_y: usize) -> Pos {
    match clamp(p, h) {
        Pos::Grid(y) => Pos::Grid((y + 1).min(max_y)),
        Pos::Hist(id, sub) => {
            let i = (id - h.id(0)) as usize;
            if sub + 1 < h.line(i).rows(cols) {
                Pos::Hist(id, sub + 1)
            } else if i + 1 < h.len() {
                Pos::Hist(id + 1, 0)
            } else {
                Pos::Grid(0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vt::Vt;

    fn term(lines: usize) -> Vt {
        let mut v = Vt::new(10, 3, 1 << 20);
        for i in 0..lines {
            v.feed(format!("line {i}\r\n").as_bytes());
        }
        v
    }

    #[test]
    fn scrolling_back_walks_history_then_returns_live() {
        let v = term(6);
        let t = &v.t;
        assert_eq!(t.history.len(), 4);
        let mut view = View::new(Family::Terminus, 0, None);
        assert_eq!(view.rows(t, 3), [Src::Grid(0), Src::Grid(1), Src::Grid(2)]);
        view.scroll(true, 2, t, 3);
        assert!(!view.live());
        assert_eq!(
            view.rows(t, 3),
            [Src::Hist(2, 0), Src::Hist(3, 0), Src::Grid(0)]
        );
        view.scroll(true, 100, t, 3);
        assert_eq!(view.rows(t, 1), [Src::Hist(0, 0)]);
        view.scroll(false, 100, t, 3);
        assert!(view.live());
        assert!(view.follow);
    }

    #[test]
    fn long_lines_scroll_a_row_at_a_time() {
        let mut v = Vt::new(4, 2, 1 << 20);
        v.feed(b"abcdefgh\r\nx\r\ny");
        let t = &v.t;
        let mut view = View::new(Family::Terminus, 0, None);
        view.scroll(true, 1, t, 2);
        assert_eq!(view.rows(t, 2), [Src::Hist(0, 1), Src::Grid(0)]);
    }

    #[test]
    fn the_window_follows_the_cursor() {
        let mut view = View::new(Family::Terminus, 0, Some((80, 24)));
        view.follow_cursor((70, 20), (80, 24), (53, 12));
        // Two cells of margin past the cursor.
        assert_eq!(view.pan_x, 20);
        assert_eq!(view.top, Pos::Grid(11));
        view.follow_cursor((0, 0), (80, 24), (53, 12));
        assert_eq!((view.pan_x, view.top), (0, Pos::Grid(0)));
    }
}
