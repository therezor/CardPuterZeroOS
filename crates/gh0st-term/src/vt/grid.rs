//! The visible screen: rows of cells, which rows changed, and the moves a
//! terminal makes on them.

use super::cell::{Cell, Pen};

#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// The line goes on in the next row: autowrap put it there.
    pub wrapped: bool,
}

impl Row {
    pub fn new(cols: usize, fill: Cell) -> Row {
        Row {
            cells: vec![fill; cols],
            wrapped: false,
        }
    }

    pub fn reset(&mut self, fill: Cell) {
        self.cells.fill(fill);
        self.wrapped = false;
    }

    /// Cells up to the last one that shows something.
    pub fn used(&self) -> usize {
        self.cells
            .iter()
            .rposition(|c| !c.is_blank())
            .map_or(0, |i| i + 1)
    }
}

pub struct Grid {
    pub rows: Vec<Row>,
    pub cols: usize,
    /// Rows changed since the last draw.
    pub dirty: Vec<bool>,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Grid {
        Grid {
            rows: (0..rows).map(|_| Row::new(cols, Cell::BLANK)).collect(),
            cols,
            dirty: vec![true; rows],
        }
    }

    pub fn height(&self) -> usize {
        self.rows.len()
    }

    pub fn mark(&mut self, y: usize) {
        if let Some(d) = self.dirty.get_mut(y) {
            *d = true;
        }
    }

    pub fn mark_range(&mut self, top: usize, bot: usize) {
        for d in &mut self.dirty[top..=bot.min(self.rows.len() - 1)] {
            *d = true;
        }
    }

    pub fn mark_all(&mut self) {
        self.dirty.fill(true);
    }

    pub fn clear(&mut self, pen: &Pen) {
        let fill = Cell::blank(pen);
        for r in &mut self.rows {
            r.reset(fill);
        }
        self.mark_all();
    }

    /// Moves rows `top..=bot` up by `n`; blank rows come in at the bottom.
    pub fn scroll_up(&mut self, top: usize, bot: usize, n: usize, pen: &Pen) {
        let n = n.min(bot + 1 - top);
        self.rows[top..=bot].rotate_left(n);
        let fill = Cell::blank(pen);
        for r in &mut self.rows[bot + 1 - n..=bot] {
            r.reset(fill);
        }
        self.mark_range(top, bot);
    }

    /// Moves rows `top..=bot` down by `n`; blank rows come in at the top.
    pub fn scroll_down(&mut self, top: usize, bot: usize, n: usize, pen: &Pen) {
        let n = n.min(bot + 1 - top);
        self.rows[top..=bot].rotate_right(n);
        let fill = Cell::blank(pen);
        for r in &mut self.rows[top..top + n] {
            r.reset(fill);
        }
        self.mark_range(top, bot);
    }

    /// Blanks cells `x0..x1` of row `y`, splitting no wide char in half.
    pub fn erase(&mut self, y: usize, x0: usize, x1: usize, pen: &Pen) {
        let x1 = x1.min(self.cols);
        if x0 >= x1 {
            return;
        }
        self.unsplit(y, x0);
        self.unsplit(y, x1 - 1);
        self.rows[y].cells[x0..x1].fill(Cell::blank(pen));
        self.mark(y);
    }

    /// Before cell `x` changes: if it is half of a wide char, both halves
    /// become blanks so no half char is left behind.
    pub fn unsplit(&mut self, y: usize, x: usize) {
        let cols = self.cols;
        let cells = &mut self.rows[y].cells;
        let c = cells[x];
        let pair = if c.is_spacer() && x > 0 && cells[x - 1].is_wide() {
            x - 1
        } else if c.is_wide() && x + 1 < cols && cells[x + 1].is_spacer() {
            x
        } else {
            return;
        };
        let blank = Cell::blank(&cells[pair].pen());
        cells[pair] = blank;
        cells[pair + 1] = blank;
    }

    /// Inserts `n` blanks at `x`, pushing the rest of the row right.
    pub fn insert_cells(&mut self, y: usize, x: usize, n: usize, pen: &Pen) {
        let cols = self.cols;
        if x >= cols {
            return;
        }
        let n = n.min(cols - x);
        // A wide char that starts at x moves whole; one that starts before
        // x is split.
        if self.rows[y].cells[x].is_spacer() {
            self.unsplit(y, x);
        }
        let cells = &mut self.rows[y].cells;
        cells[x..].rotate_right(n);
        cells[x..x + n].fill(Cell::blank(pen));
        // A wide char pushed to the edge loses its spacer.
        if cells[cols - 1].is_wide() {
            cells[cols - 1] = Cell::blank(&cells[cols - 1].pen());
        }
        self.mark(y);
    }

    /// Deletes `n` cells at `x`, pulling the rest left; blanks fill the end.
    pub fn delete_cells(&mut self, y: usize, x: usize, n: usize, pen: &Pen) {
        let cols = self.cols;
        if x >= cols {
            return;
        }
        let n = n.min(cols - x);
        self.unsplit(y, x);
        self.unsplit(y, x + n - 1);
        let cells = &mut self.rows[y].cells;
        cells[x..].rotate_left(n);
        cells[cols - n..].fill(Cell::blank(pen));
        self.mark(y);
    }

    /// New size. Rows keep their cells from the left; new space is blank.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        for r in &mut self.rows {
            r.cells.resize(cols, Cell::BLANK);
            if r.cells.last().is_some_and(|c| c.is_wide()) {
                let last = r.cells.len() - 1;
                r.cells[last] = Cell::BLANK;
            }
            if cols < self.cols {
                r.wrapped = false;
            }
        }
        self.rows.resize_with(rows, || Row::new(cols, Cell::BLANK));
        self.cols = cols;
        self.dirty = vec![true; rows];
    }

    /// Text of row `y`, for tests and copy.
    #[cfg(test)]
    pub fn text(&self, y: usize) -> String {
        let r = &self.rows[y];
        r.cells[..r.used()]
            .iter()
            .filter(|c| !c.is_spacer())
            .map(|c| c.ch())
            .collect()
    }
}

/// Rewraps `rows` at `cols`: rows joined by autowrap become one line again
/// and wrap at the new width. Returns the rows and where the cursor lands.
pub fn reflow(rows: &[Row], cols: usize, cursor: (usize, usize)) -> (Vec<Row>, (usize, usize)) {
    // Logical lines, each with the cursor's offset if it is on it.
    let mut lines: Vec<(Vec<Cell>, Option<usize>)> = Vec::new();
    let mut line = Vec::new();
    let mut at = None;
    for (y, r) in rows.iter().enumerate() {
        if y == cursor.1 {
            at = Some(line.len() + cursor.0);
        }
        let n = if r.wrapped { r.cells.len() } else { r.used() };
        line.extend_from_slice(&r.cells[..n]);
        if !r.wrapped || y + 1 == rows.len() {
            lines.push((std::mem::take(&mut line), at.take()));
        }
    }
    let mut out: Vec<Row> = Vec::new();
    let mut cur = (0, 0);
    for (cells, at) in lines {
        let mut row = Row::new(0, Cell::BLANK);
        let mut i = 0;
        while i < cells.len() {
            let w = if cells[i].is_wide() { 2 } else { 1 };
            if row.cells.len() + w > cols {
                row.cells.resize(cols, Cell::BLANK);
                row.wrapped = true;
                out.push(std::mem::replace(&mut row, Row::new(0, Cell::BLANK)));
            }
            if at == Some(i) {
                cur = (row.cells.len(), out.len());
            }
            row.cells.push(cells[i]);
            if w == 2 {
                row.cells
                    .push(cells.get(i + 1).copied().unwrap_or(Cell::BLANK));
            }
            i += w;
        }
        if let Some(a) = at.filter(|&a| a >= cells.len()) {
            // Past the text: count on from its end.
            let past = row.cells.len() + (a - cells.len());
            cur = (past % cols, out.len() + past / cols);
        }
        row.cells.resize(cols, Cell::BLANK);
        out.push(row);
    }
    // Blank rows below the cursor carry nothing.
    while out.len() > cur.1 + 1 && out.last().is_some_and(|r| r.used() == 0) {
        out.pop();
    }
    while out.len() <= cur.1 {
        out.push(Row::new(cols, Cell::BLANK));
    }
    (out, cur)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(lines: &[&str]) -> Grid {
        let mut g = Grid::new(6, lines.len());
        for (y, l) in lines.iter().enumerate() {
            for (x, ch) in l.chars().enumerate() {
                g.rows[y].cells[x] = Cell::new(ch, &Pen::DEFAULT);
            }
        }
        g
    }

    #[test]
    fn scroll_inside_a_region() {
        let mut g = grid(&["a", "b", "c", "d"]);
        g.scroll_up(1, 2, 1, &Pen::DEFAULT);
        assert_eq!(
            (0..4).map(|y| g.text(y)).collect::<Vec<_>>(),
            ["a", "c", "", "d"]
        );
        g.scroll_down(0, 3, 2, &Pen::DEFAULT);
        assert_eq!(
            (0..4).map(|y| g.text(y)).collect::<Vec<_>>(),
            ["", "", "a", "c"]
        );
    }

    #[test]
    fn insert_and_delete_cells() {
        let mut g = grid(&["abcdef"]);
        g.insert_cells(0, 1, 2, &Pen::DEFAULT);
        assert_eq!(g.text(0), "a  bcd");
        g.delete_cells(0, 0, 3, &Pen::DEFAULT);
        assert_eq!(g.text(0), "bcd");
    }

    fn wrapped(lines: &[(&str, bool)], cols: usize) -> Vec<Row> {
        lines
            .iter()
            .map(|(s, w)| {
                let mut r = Row::new(cols, Cell::BLANK);
                for (x, ch) in s.chars().enumerate() {
                    r.cells[x] = Cell::new(ch, &Pen::DEFAULT);
                }
                r.wrapped = *w;
                r
            })
            .collect()
    }

    fn texts(rows: &[Row]) -> Vec<String> {
        let g = Grid {
            cols: rows[0].cells.len(),
            dirty: vec![false; rows.len()],
            rows: rows.to_vec(),
        };
        (0..rows.len()).map(|y| g.text(y)).collect()
    }

    #[test]
    fn reflow_joins_and_rewraps() {
        let rows = wrapped(&[("abcd", true), ("ef", false), ("$ ", false)], 4);
        let (out, cur) = reflow(&rows, 6, (2, 2));
        assert_eq!(texts(&out), ["abcdef", "$"]);
        assert_eq!(cur, (2, 1));
        let (out, cur) = reflow(&out, 3, cur);
        assert_eq!(texts(&out), ["abc", "def", "$"]);
        // "def" ends its line, so only the first row wraps.
        assert!(out[0].wrapped && !out[1].wrapped && !out[2].wrapped);
        assert_eq!(cur, (2, 2));
    }

    #[test]
    fn reflow_keeps_the_cursor_on_its_char() {
        let rows = wrapped(&[("hello wor", true), ("ld", false)], 9);
        let (out, cur) = reflow(&rows, 5, (1, 1));
        assert_eq!(texts(&out), ["hello", " worl", "d"]);
        assert_eq!(out[cur.1].cells[cur.0].ch(), 'd');
    }

    #[test]
    fn erasing_half_a_wide_char_erases_both() {
        let mut g = Grid::new(4, 1);
        g.rows[0].cells[1] = Cell::wide('漢', &Pen::DEFAULT);
        g.rows[0].cells[2] = Cell::spacer(&Pen::DEFAULT);
        g.erase(0, 2, 3, &Pen::DEFAULT);
        assert!(g.rows[0].cells.iter().all(|c| c.is_blank()));
    }
}
