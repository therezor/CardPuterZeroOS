//! The pages. Lists come from the keyring each time a page opens or comes
//! back; a prompt's end arrives on the bus fd, never by waiting for it.

use gh0st_app::prelude::*;

use crate::backend::{AccessRow, ItemRow, OUR_SCHEMA, Shared, State, app_label};

const APP_ICON: &str = "key-round";
const ASK_DELETE: u32 = 1;

/// A prompt the page started and waits on.
struct Waiting {
    path: String,
    watch: Option<Watch>,
}

/// How starting a prompt went.
enum Begun {
    /// Nothing to ask, or it already ended: whether it was dismissed.
    Done(bool),
    Waiting(Waiting),
    Failed,
}

/// Starts a prompt from `start` and watches for its end, which may have
/// come already: the reply and the signal can arrive together.
fn begin(
    kr: &Shared,
    cx: &mut Cx,
    start: impl FnOnce(&mut dyn crate::backend::Backend) -> Result<Option<String>, String>,
) -> Begun {
    let started = start(&mut **kr.borrow_mut());
    match started {
        Ok(Some(path)) => {
            let w = Waiting { path, watch: None };
            if let Some(d) = ended(kr, &w) {
                return Begun::Done(d);
            }
            let watch = kr.borrow().fd().map(|fd| cx.watch(fd));
            Begun::Waiting(Waiting { watch, ..w })
        }
        Ok(None) => Begun::Done(false),
        Err(e) => {
            cx.toast_tone(Tone::Critical, &e);
            Begun::Failed
        }
    }
}

/// Whether the prompt ended, and if so whether it was dismissed.
fn ended(kr: &Shared, w: &Waiting) -> Option<bool> {
    kr.borrow_mut()
        .completed()
        .into_iter()
        .find(|(p, _)| *p == w.path)
        .map(|(_, d)| d)
}

fn finish(cx: &mut Cx, w: Waiting) {
    if let Some(id) = w.watch {
        cx.unwatch(id);
    }
}

pub struct Home {
    kr: Shared,
    list: List,
    state: State,
    items: Vec<ItemRow>,
    access: usize,
    waiting: Option<Waiting>,
}

impl Home {
    pub fn new(kr: Shared) -> Home {
        Home {
            kr,
            list: List::new(),
            state: State::Absent,
            items: Vec::new(),
            access: 0,
            waiting: None,
        }
    }

    fn reload(&mut self) {
        let mut kr = self.kr.borrow_mut();
        self.state = kr.state();
        self.items = kr.items();
        self.access = kr.access().len();
    }

    fn counts(&self) -> (usize, usize) {
        let wifi = self.items.iter().filter(|i| i.wifi()).count();
        (self.items.len() - wifi, wifi)
    }
}

impl Page for Home {
    fn title(&self) -> &str {
        "Keyring"
    }

    fn icon(&self) -> &str {
        APP_ICON
    }

    fn about(&self) -> &str {
        "Apps keep their passwords and tokens here, encrypted with your password. An app reads its own; to read another's it needs your yes."
    }

    fn status(&self) -> Option<(&str, Tone)> {
        match self.state {
            State::Open => Some(("OPEN", Tone::Ok)),
            State::Locked => Some(("LOCKED", Tone::Warning)),
            State::Absent => None,
        }
    }

    fn start(&mut self, _: &mut Cx) {
        self.reload();
    }

    fn resume(&mut self, _: &mut Cx) {
        self.reload();
    }

    fn bars(&self) -> Bars {
        match self.state {
            State::Open => Bars::top()
                .chip(5, Chip::new("lock", "Lock"))
                .chip(7, Chip::new("plus", "Add")),
            State::Locked => Bars::top().chip(6, Chip::new("lock-open", "Unlock").main()),
            State::Absent => Bars::top(),
        }
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match (self.state, key) {
            (State::Open, Key::Soft(5)) => {
                self.kr.borrow_mut().lock();
                self.reload();
                return;
            }
            (State::Open, Key::Soft(7)) => {
                cx.push(Add::new(self.kr.clone()));
                return;
            }
            (State::Locked, Key::Soft(6)) => {
                if self.waiting.is_none() {
                    if let Begun::Waiting(w) = begin(&self.kr, cx, |b| b.unlock()) {
                        self.waiting = Some(w);
                    }
                }
                self.reload();
                return;
            }
            _ => {}
        }
        if self.state != State::Open {
            return;
        }
        if let Some(Action::Open(i)) = self.list.key(key) {
            match i {
                0 => cx.push(Items::new(self.kr.clone(), false)),
                1 => cx.push(Items::new(self.kr.clone(), true)),
                _ => cx.push(Access::new(self.kr.clone())),
            }
        }
    }

    fn ready(&mut self, _: Watch, cx: &mut Cx) {
        let Some(w) = &self.waiting else {
            return;
        };
        if ended(&self.kr, w).is_some() {
            if let Some(w) = self.waiting.take() {
                finish(cx, w);
            }
            self.reload();
            cx.redraw();
        }
    }

    fn stop(&mut self, cx: &mut Cx) {
        if let Some(w) = self.waiting.take() {
            finish(cx, w);
        }
    }

    fn draw(&self, ui: &mut Ui) {
        match self.state {
            State::Absent => ui.message("key-round", "The keyring isn't running."),
            State::Locked => ui.message("lock", "Locked. Unlock it with your password."),
            State::Open => {
                let (pw, wifi) = self.counts();
                let (pw, wifi, access) =
                    (pw.to_string(), wifi.to_string(), self.access.to_string());
                ui.list(
                    &self.list,
                    &[
                        Row::link("Passwords").icon("key-round").value(&pw),
                        Row::link("Wi-Fi").icon("wifi").value(&wifi),
                        Row::link("App access").icon("shield").value(&access),
                    ],
                );
            }
        }
    }
}

/// The items of one kind: passwords (by app) or Wi-Fi.
pub struct Items {
    kr: Shared,
    wifi: bool,
    list: List,
    rows: Vec<(ItemRow, String)>,
}

impl Items {
    pub fn new(kr: Shared, wifi: bool) -> Items {
        Items {
            kr,
            wifi,
            list: List::new(),
            rows: Vec::new(),
        }
    }

    fn reload(&mut self) {
        let mut rows: Vec<(ItemRow, String)> = self
            .kr
            .borrow_mut()
            .items()
            .into_iter()
            .filter(|i| i.wifi() == self.wifi)
            .map(|i| {
                let app = app_label(&i.app);
                (i, app)
            })
            .collect();
        // By app, then by name: an app's secrets sit together.
        rows.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.label.cmp(&b.0.label)));
        self.rows = rows;
    }
}

impl Page for Items {
    fn title(&self) -> &str {
        if self.wifi { "Wi-Fi" } else { "Passwords" }
    }

    fn icon(&self) -> &str {
        if self.wifi { "wifi" } else { "key-round" }
    }

    fn start(&mut self, _: &mut Cx) {
        self.reload();
    }

    fn resume(&mut self, _: &mut Cx) {
        self.reload();
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if let Some(Action::Open(i)) = self.list.key(key) {
            if let Some((row, app)) = self.rows.get(i) {
                cx.push(Item::new(self.kr.clone(), row.clone(), app.clone()));
            }
        }
    }

    fn draw(&self, ui: &mut Ui) {
        if self.rows.is_empty() {
            let empty = if self.wifi {
                "No Wi-Fi passwords yet."
            } else {
                "No passwords yet. Add one on 7 at home."
            };
            ui.message(self.icon(), empty);
            return;
        }
        let rows: Vec<Row> = self
            .rows
            .iter()
            .map(|(r, app)| Row::link(&r.label).value(app))
            .collect();
        ui.list(&self.list, &rows);
    }
}

/// One item: its name, who stored it, and the secret on request.
pub struct Item {
    kr: Shared,
    row: ItemRow,
    app: String,
    list: List,
    shown: Option<String>,
    waiting: Option<(Waiting, bool)>,
}

impl Item {
    pub fn new(kr: Shared, row: ItemRow, app: String) -> Item {
        Item {
            kr,
            row,
            app,
            list: List::new(),
            shown: None,
            waiting: None,
        }
    }

    fn show(&mut self, cx: &mut Cx) {
        let item = self.row.path.clone();
        // The shell asks for the password even while the keyring is open.
        match begin(&self.kr, cx, |b| b.reveal(&item)) {
            Begun::Waiting(w) => self.waiting = Some((w, false)),
            Begun::Done(false) => self.read(cx),
            Begun::Done(true) => cx.toast("Not allowed"),
            Begun::Failed => {}
        }
    }

    fn read(&mut self, cx: &mut Cx) {
        match self.kr.borrow_mut().secret(&self.row.path) {
            Some(mut s) => {
                self.shown = Some(String::from_utf8_lossy(&s).into_owned());
                gh0st_core::password::wipe(&mut s);
            }
            None => cx.toast("Not shown"),
        }
    }
}

impl Drop for Item {
    fn drop(&mut self) {
        if let Some(s) = &mut self.shown {
            // SAFETY: zero bytes keep the string valid UTF-8.
            gh0st_core::password::wipe(unsafe { s.as_bytes_mut() });
        }
    }
}

impl Page for Item {
    fn title(&self) -> &str {
        &self.row.label
    }

    fn icon(&self) -> &str {
        if self.row.wifi() { "wifi" } else { "key-round" }
    }

    fn bars(&self) -> Bars {
        let show = if self.shown.is_some() {
            Chip::new("eye-off", "Hide")
        } else {
            Chip::new("eye", "Show")
        };
        Bars::top()
            .chip(5, show)
            .chip(7, Chip::new("trash-2", "Delete").danger())
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match key {
            Key::Soft(5) if self.shown.is_some() => {
                if let Some(s) = &mut self.shown {
                    // SAFETY: zero bytes keep the string valid UTF-8.
                    gh0st_core::password::wipe(unsafe { s.as_bytes_mut() });
                }
                self.shown = None;
            }
            Key::Soft(5) if self.waiting.is_none() => self.show(cx),
            Key::Soft(7) => cx.ask(
                ASK_DELETE,
                Dialog::confirm(
                    "Delete it?",
                    &format!("{} can't use it after this.", self.app),
                )
                .yes("delete")
                .danger(),
            ),
            k => {
                self.list.key(k);
            }
        }
    }

    fn answer(&mut self, id: u32, answer: Answer, cx: &mut Cx) {
        if id != ASK_DELETE || answer == Answer::No || self.waiting.is_some() {
            return;
        }
        let item = self.row.path.clone();
        match begin(&self.kr, cx, |b| b.delete(&item)) {
            Begun::Waiting(w) => self.waiting = Some((w, true)),
            Begun::Done(false) => cx.back(),
            Begun::Done(true) => cx.toast("Not allowed"),
            Begun::Failed => {}
        }
    }

    fn ready(&mut self, _: Watch, cx: &mut Cx) {
        let Some((w, delete)) = &self.waiting else {
            return;
        };
        let (Some(dismissed), delete) = (ended(&self.kr, w), *delete) else {
            return;
        };
        if let Some((w, _)) = self.waiting.take() {
            finish(cx, w);
        }
        match (dismissed, delete) {
            (true, _) => cx.toast("Not allowed"),
            (false, true) => cx.back(),
            (false, false) => self.read(cx),
        }
        cx.redraw();
    }

    fn stop(&mut self, cx: &mut Cx) {
        if let Some((w, _)) = self.waiting.take() {
            finish(cx, w);
        }
    }

    fn draw(&self, ui: &mut Ui) {
        let secret = match &self.shown {
            Some(s) => s.as_str(),
            None => "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}",
        };
        let kind = if self.row.wifi() { "Wi-Fi" } else { "Password" };
        ui.list(
            &self.list,
            &[Row::info("Stored by", &self.app), Row::info(kind, secret)],
        );
    }
}

/// Which apps may read secrets they didn't store.
pub struct Access {
    kr: Shared,
    list: List,
    rows: Vec<(AccessRow, String)>,
}

impl Access {
    pub fn new(kr: Shared) -> Access {
        Access {
            kr,
            list: List::new(),
            rows: Vec::new(),
        }
    }

    fn reload(&mut self) {
        self.rows = self
            .kr
            .borrow_mut()
            .access()
            .into_iter()
            .map(|a| {
                let app = app_label(&a.app);
                (a, app)
            })
            .collect();
        self.rows
            .sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.label.cmp(&b.0.label)));
    }
}

impl Page for Access {
    fn title(&self) -> &str {
        "App access"
    }

    fn icon(&self) -> &str {
        "shield"
    }

    fn start(&mut self, _: &mut Cx) {
        self.reload();
    }

    fn bars(&self) -> Bars {
        let b = Bars::top();
        if self.rows.is_empty() {
            return b;
        }
        b.chip(6, Chip::new("x", "Revoke").danger())
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Soft(6) {
            if let Some((a, app)) = self.rows.get(self.list.selected()) {
                let done = self.kr.borrow_mut().revoke(&a.app, &a.item);
                cx.toast(&if done {
                    format!("{app} asks again next time")
                } else {
                    "Couldn't revoke".into()
                });
            }
            self.reload();
            let last = self.rows.len().saturating_sub(1);
            self.list.select(self.list.selected().min(last));
            return;
        }
        self.list.key(key);
    }

    fn draw(&self, ui: &mut Ui) {
        if self.rows.is_empty() {
            ui.message("shield", "No app reads another's secrets.");
            return;
        }
        let rows: Vec<Row> = self
            .rows
            .iter()
            .map(|(a, app)| Row::info(app, &a.label))
            .collect();
        ui.list(&self.list, &rows);
    }
}

/// A password typed by hand.
pub struct Add {
    kr: Shared,
    form: Form,
}

impl Add {
    pub fn new(kr: Shared) -> Add {
        Add {
            kr,
            form: Form::new().text("Name").password("Password"),
        }
    }
}

impl Page for Add {
    fn title(&self) -> &str {
        "Add a password"
    }

    fn icon(&self) -> &str {
        "plus"
    }

    fn typing(&self) -> bool {
        true
    }

    fn bars(&self) -> Bars {
        Bars::top().hints(self.form.hints("back", "save"))
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if self.form.key(key) != Some(Edit::Submit) {
            return;
        }
        self.form.clear_errors();
        if self.form.value(0).trim().is_empty() {
            self.form.error(0, "Give it a name");
            self.form.set_focus(0);
            return;
        }
        if self.form.value(1).is_empty() {
            self.form.error(1, "The password is empty");
            self.form.set_focus(1);
            return;
        }
        let name = self.form.take(0);
        let mut pw = self.form.take(1);
        let attrs = [("xdg:schema", OUR_SCHEMA), ("name", name.trim())];
        let r = self
            .kr
            .borrow_mut()
            .store(name.trim(), &attrs, pw.as_bytes());
        // SAFETY: zero bytes keep the string valid UTF-8.
        gh0st_core::password::wipe(unsafe { pw.as_bytes_mut() });
        match r {
            Ok(()) => {
                cx.toast("Saved");
                cx.back();
            }
            Err(e) => cx.toast_tone(Tone::Critical, &e),
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.form(&self.form);
    }
}
