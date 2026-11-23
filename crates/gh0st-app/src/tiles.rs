//! A grid of tiles: a gallery of pictures, a folder of images. It keeps
//! the selection and the scroll and lays the tiles out; the page draws
//! what goes in each one. The arrows move it: bind them as commands and call
//! [`Tiles::step`].
//!
//! ```
//! use gh0st_app::Tiles;
//! use gh0st_app::gfx::Rect;
//!
//! let mut grid = Tiles::new(5, 3);
//! grid.set_len(12);
//! grid.step(1, 1);
//! assert_eq!(grid.at(), 6);
//! let shown: Vec<usize> = grid
//!     .layout(Rect::new(0, 18, 320, 132), (53, 40), 3)
//!     .map(|(i, _)| i)
//!     .collect();
//! assert_eq!(shown, (0..12).collect::<Vec<_>>());
//! ```

use gh0st_ui::Rect;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tiles {
    len: usize,
    at: usize,
    /// The first row on screen.
    top: usize,
    cols: usize,
    rows: usize,
}

impl Tiles {
    /// A grid `cols` wide with `rows` on screen at once.
    pub fn new(cols: usize, rows: usize) -> Tiles {
        Tiles {
            len: 0,
            at: 0,
            top: 0,
            cols: cols.max(1),
            rows: rows.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The selected tile.
    pub fn at(&self) -> usize {
        self.at
    }

    /// How many tiles there are now: the selection stays on one that is.
    pub fn set_len(&mut self, n: usize) {
        self.len = n;
        self.select(self.at);
    }

    /// Selects tile `i`, or the last, and scrolls it into view.
    pub fn select(&mut self, i: usize) {
        self.at = i.min(self.len.saturating_sub(1));
        let row = self.at / self.cols;
        if row < self.top {
            self.top = row;
        } else if row >= self.top + self.rows {
            self.top = row + 1 - self.rows;
        }
    }

    /// Moves by `dx` tiles across and `dy` rows down. Across runs on into
    /// the next row; down stops at the last tile. True when it moved.
    pub fn step(&mut self, dx: isize, dy: isize) -> bool {
        if self.len == 0 {
            return false;
        }
        let last = self.len as isize - 1;
        let to = (self.at as isize + dx + dy * self.cols as isize).clamp(0, last);
        let moved = to as usize != self.at;
        self.select(to as usize);
        moved
    }

    /// The first tile on screen and how many fit: for a scrollbar.
    pub fn window(&self) -> (usize, usize) {
        (self.top * self.cols, self.rows * self.cols)
    }

    /// The tiles on screen, each with its place: `tile` sized, `gap` apart,
    /// the grid centred across `area` and starting at its top.
    pub fn layout(
        &self,
        area: Rect,
        tile: (i32, i32),
        gap: i32,
    ) -> impl Iterator<Item = (usize, Rect)> {
        let cols = self.cols as i32;
        let w = cols * tile.0 + (cols - 1) * gap;
        let x0 = area.x + (area.w - w) / 2;
        let first = self.top * self.cols;
        let end = (first + self.rows * self.cols).min(self.len);
        let cols = self.cols;
        (first..end).map(move |i| {
            let k = i - first;
            let (col, row) = ((k % cols) as i32, (k / cols) as i32);
            let r = Rect::new(
                x0 + col * (tile.0 + gap),
                area.y + row * (tile.1 + gap),
                tile.0,
                tile.1,
            );
            (i, r)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moving_down_scrolls_and_stops_at_the_last_tile() {
        let mut g = Tiles::new(5, 3);
        g.set_len(17);
        assert!(g.step(0, 3));
        assert_eq!((g.at(), g.window()), (15, (5, 15)));
        g.step(4, 0);
        assert_eq!(g.at(), 16, "across stops at the last tile");
        assert!(!g.step(0, 1));
        g.step(0, -3);
        assert_eq!((g.at(), g.window().0), (1, 0));
    }

    #[test]
    fn a_shorter_list_keeps_the_selection_on_a_tile() {
        let mut g = Tiles::new(5, 3);
        g.set_len(9);
        g.select(8);
        g.set_len(4);
        assert_eq!(g.at(), 3);
        g.set_len(0);
        assert!(!g.step(1, 0));
    }

    #[test]
    fn tiles_sit_centred_in_rows() {
        let mut g = Tiles::new(2, 2);
        g.set_len(3);
        let r: Vec<_> = g.layout(Rect::new(0, 10, 100, 100), (40, 30), 4).collect();
        assert_eq!(r[0].1, Rect::new(8, 10, 40, 30));
        assert_eq!(r[1].1, Rect::new(52, 10, 40, 30));
        assert_eq!(r[2].1, Rect::new(8, 44, 40, 30));
    }
}
