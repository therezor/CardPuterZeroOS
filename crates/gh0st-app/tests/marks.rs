//! A long stopping list with marked rows: paging, ends, and the design lint
//! on every palette. A dialog with a long value shows its tail.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use std::cell::Cell;
use std::rc::Rc;

const ASK: u32 = 1;

struct Folder {
    list: List,
    names: Vec<String>,
    marked: Vec<bool>,
    at: Rc<Cell<usize>>,
}

impl Page for Folder {
    fn title(&self) -> &str {
        "Folder"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match key {
            Key::Char(' ') => {
                let i = self.list.selected();
                self.marked[i] = !self.marked[i];
            }
            Key::Char('r') => {
                let long = "a very long file name that goes on past the field.tar.gz";
                cx.ask(ASK, Dialog::text("Rename", "").value(long));
            }
            k => {
                self.list.key(k);
            }
        }
        self.at.set(self.list.selected());
    }

    fn draw(&self, ui: &mut Ui) {
        let rows: Vec<Row> = self
            .names
            .iter()
            .zip(&self.marked)
            .map(|(n, &m)| Row::info(n, "3.4 MB").icon("file").marked(m))
            .collect();
        ui.list(&self.list, &rows);
    }
}

fn folder() -> (Driver, Rc<Cell<usize>>) {
    let at = Rc::new(Cell::new(0));
    let names: Vec<String> = (1..=30).map(|i| format!("track {i:02}.flac")).collect();
    let d = Driver::new(Folder {
        list: List::stopping(),
        marked: vec![false; names.len()],
        names,
        at: at.clone(),
    });
    (d, at)
}

#[test]
fn pages_stop_at_the_ends() {
    let (mut d, at) = folder();
    d.frame();
    d.keys("f");
    assert_eq!(at.get(), 0, "Up on the first row stays");
    d.keys("pagedown");
    assert_eq!(at.get(), 5, "a page is the 5 rows on screen");
    for _ in 0..10 {
        d.keys("pagedown");
    }
    assert_eq!(at.get(), 29);
    d.keys("x");
    assert_eq!(at.get(), 29, "Down on the last row stays");
    d.keys("pageup");
    assert_eq!(at.get(), 24);
}

#[test]
fn marked_rows_keep_the_lint_on_every_palette() {
    let (mut d, _) = folder();
    d.frame();
    d.keys("space").keys("x").keys("x").keys("space");
    for p in gh0st_app::gfx::theme::PALETTES {
        d.palette(p.name);
        let found = d.lint();
        assert!(found.is_empty(), "{}: {found:?}", p.name);
    }
}

#[test]
fn a_long_value_keeps_the_lint() {
    let (mut d, _) = folder();
    d.keys("r");
    assert_eq!(d.dialog().map(|x| x.0), Some("Rename"));
    d.keys("type xyz");
    let found = d.lint();
    assert!(found.is_empty(), "{found:?}");
}
