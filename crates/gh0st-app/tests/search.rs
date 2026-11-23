//! Search on a list page: 6 or `s` starts it, digits then type, Fn+X moves,
//! Enter picks the match, and Esc leaves search before it leaves the page.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use std::cell::RefCell;
use std::rc::Rc;

const FRUIT: &[(&str, &str)] = &[
    ("Apple", "red or green"),
    ("Banana", "yellow, curved"),
    ("Mango", "orange inside"),
    ("Plum 4", "small, dark"),
];

struct Fruit {
    list: List,
    search: Search,
    picked: Rc<RefCell<Vec<&'static str>>>,
}

impl Fruit {
    fn shown(&self) -> Vec<usize> {
        self.search.rank(FRUIT.len(), |i| {
            self.search.score(&[FRUIT[i].0, FRUIT[i].1])
        })
    }
}

impl Page for Fruit {
    fn title(&self) -> &str {
        "Fruit"
    }

    fn bars(&self) -> Bars {
        Bars::top()
            .chip(6, Chip::new("search", "Find"))
            .search(&self.search)
    }

    fn typing(&self) -> bool {
        self.search.active()
    }

    fn back(&mut self, _: &mut Cx) -> bool {
        !self.search.cancel()
    }

    fn key(&mut self, key: Key, _: &mut Cx) {
        if !self.search.active() && matches!(key, Key::Soft(6) | Key::Char('s')) {
            self.search.start();
            return;
        }
        if self.search.key(key) == Some(Edit::Changed) {
            self.list.select(0);
            return;
        }
        let shown = self.shown();
        if let Some(Action::Open(i)) = self.list.key(key) {
            self.picked.borrow_mut().push(FRUIT[shown[i]].0);
            self.search.cancel();
        }
    }

    fn draw(&self, ui: &mut Ui) {
        let rows: Vec<Row> = self
            .shown()
            .into_iter()
            .map(|i| Row::text(FRUIT[i].0).note(FRUIT[i].1))
            .collect();
        ui.list(&self.list, &rows);
    }
}

fn start() -> (Driver, Rc<RefCell<Vec<&'static str>>>) {
    let picked = Rc::new(RefCell::new(Vec::new()));
    let page = Fruit {
        list: List::new(),
        search: Search::new(),
        picked: picked.clone(),
    };
    let mut d = Driver::new(page);
    d.frame();
    (d, picked)
}

#[test]
fn enter_picks_the_best_match_and_leaves_search() {
    let (mut d, picked) = start();
    d.keys("6\ntype an\nenter");
    // "an" starts no name; Banana and Mango have it inside, in list order.
    assert_eq!(*picked.borrow(), ["Banana"]);
    d.keys("enter");
    assert_eq!(*picked.borrow(), ["Banana", "Apple"], "search was left");
}

#[test]
fn digits_type_while_searching_and_fn_x_moves() {
    let (mut d, picked) = start();
    d.keys("s\n4\nenter");
    assert_eq!(*picked.borrow(), ["Plum 4"], "4 typed, not Esc");
    d.keys("s\ntype a\nfn+x\nenter");
    assert_eq!(*picked.borrow(), ["Plum 4", "Banana"]);
}

#[test]
fn esc_leaves_search_first_then_the_page() {
    let (mut d, picked) = start();
    d.keys("6\ntype zz");
    assert!(d.lint().is_empty(), "{:?}", d.lint());
    // A frame between keys, as a person's pace gives: the list learns its
    // new length when drawn.
    d.keys("esc").wait(100);
    d.keys("enter");
    assert_eq!(*picked.borrow(), ["Apple"], "every fruit shows again");
    d.keys("down esc").wait(800);
    assert!(d.quit());
}

#[test]
fn an_idle_search_wakes_no_more_than_the_page() {
    let (mut plain, _) = start();
    let (mut d, _) = start();
    d.keys("6\ntype m").wait(100);
    plain.wait(100);
    assert_eq!(d.idle_wakes(600_000), plain.idle_wakes(600_000));
}
