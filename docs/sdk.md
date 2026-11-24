# Writing a gh0st app

`gh0st-app` is the SDK for CardputerZero apps written in Rust. An app is a
stack of pages. Each page says which OS bars it wants, draws itself, and
answers keys. The SDK runs the event loop, maps the keyboard, draws the bars,
and sends only the changed rows to the panel.

What every screen looks like, and what every key does, is in `design.md`.
The SDK draws all of it, so an app follows it by using the SDK.

Settings (`crates/gh0st-settings`) is built on this SDK, plus `gh0st-core`
for the app list, pins and the backlight. Read it for a full app. Apps in other languages use the contract in `apps.md` directly.

The SDK draws the OS bars inside the app.

## Hello

`Cargo.toml`:

```toml
[dependencies]
gh0st-app = { git = "https://github.com/therezor/CardPuterZeroOS" }
```

`src/main.rs`:

```rust,no_run
use gh0st_app::prelude::*;

struct Hello;

impl Page for Hello {
    fn title(&self) -> &str {
        "Hello"
    }

    fn draw(&self, ui: &mut Ui) {
        ui.heading("Hello, CardputerZero");
        ui.text("This page has the OS bars. Hold Esc to leave.");
    }
}

fn main() {
    gh0st_app::run(Hello);
}
```

Run it on a Mac or Linux PC. Keys come from stdin, one per line, and each new
frame is saved as a PNG:

```sh
printf 'down esc\nwait 800\nup esc\n' | cargo run -- --sim frames
```

Build for the device. Copy `.cargo/config.toml` from this repo first. It sets
the linker and turns off the crypto extensions the CM0 lacks.

```sh
cargo build --release --target aarch64-unknown-linux-musl
```

Install the binary and a `.desktop` file, and the app shows on the home
screen:

```ini
[Desktop Entry]
Type=Application
Name=Hello
Exec=/usr/bin/hello
Icon=gh0st:info
```

Put the `.desktop` file in `/usr/share/gh0st/applications`, or in
`/usr/share/APPLaunch/applications` to show under the stock launcher too.
`apps.md` lists every key the file can have.

## The screen

The panel is 320x170. A page picks its OS bars, and the rows a bar doesn't
use belong to the page:

| Bars | The page gets |
|---|---|
| `Bars::top()`, the default | 320x132, between the top bar and the action bar |
| `Bars::top_only()` | 320x152, below the top bar, for a page that takes every key |
| `Bars::actions()` | 320x150, above the action bar |
| `Bars::none()` | all 320x170 |
| `Bars::full()` | all 320x170, with the menu chip over the right rail's foot |
| `Bars::actions().over()` | all 320x170, with the action bar over its foot |

`Bars::full()` is for a page that wants the whole screen and its menu on 8,
such as a viewfinder. The SDK draws the ≡ chip in the bottom right corner,
`theme().m.rail_w` wide, so the page keeps that corner for it: a side rail,
as the camera has.

`.over()` lays the action bar on a translucent strip over the page, which
keeps all 170 rows: a viewfinder that wants its chips and its whole
picture. Leave the last `hint_h` rows free of marks, so the chips read.

The top bar shows where you are and the device. The action bar has the page's
chips over keys 5 to 7 and the menu on 8, or key hints while the page types.
A page can change its bars at any time, for example to hide them while a
video plays:

```rust
use gh0st_app::prelude::*;

struct Player {
    playing: bool,
}

impl Page for Player {
    fn title(&self) -> &str {
        "Player"
    }

    fn icon(&self) -> &str {
        "square-terminal"
    }

    fn bars(&self) -> Bars {
        if self.playing { Bars::none() } else { Bars::top() }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.message("square-terminal", "Nothing playing");
    }
}
```

The top bar shows the page's `icon` and `title`. On a page above the first,
the app's name follows in grey, or the page's `context`. `status` puts one
word of the app's own on the right, in a tone: `("REC", Tone::Critical)`.
`tabs` numbers the app's sessions and underlines the front one.

## Pages

The page passed to `run` is the app. `cx.push(page)` opens another page on top,
and Esc closes the top page. When the last page closes, the app exits.

A page needs `title` and `draw`. The other methods have defaults:

| Method | Called when |
|---|---|
| `bars` | Before each frame. The default is the top bar and the action bar |
| `icon`, `context`, `status`, `tabs` | Before each frame, for the top bar |
| `key` | A key is pressed. Esc and 4 go to `back` instead |
| `back` | Esc or 4 is pressed. Return false to stay, say after leaving a mode |
| `typing` | Before each key. Return true while a text field has focus |
| `start` | The page opens |
| `resume` | The page above it closed |
| `timer` | A timer from `cx.after` or `cx.every` fires |
| `job` | A process from `cx.spawn` prints a line or exits |
| `stop` | The page closes or the app quits. Save state here |

After every event the SDK calls `draw` and compares the frame with the last
one. Only rows that changed go over SPI. A page never asks for a redraw and
never keeps a timer to animate an idle screen.

On the first page, Esc closes the app only when held for 0.7 s: a tap shows
"Hold to exit", and the toast fills as the key is held. If the app hangs, the
shell takes over: held for 3 seconds, Esc sends SIGTERM. The SDK then
calls `stop` on every page, top first, and exits.

## Keys

| Key | Physical key |
|---|---|
| `Key::Up`, `Down`, `Left`, `Right` | Fn+F, X, Z, C. Bare F, X, Z and C too, unless the page is typing |
| `Key::Enter`, `Tab`, `Backspace`, `Delete` | The key of that name |
| `Key::Back` | Esc, and 4 while not typing. It goes to `Page::back`, not `Page::key` |
| `Key::Soft(4)` to `Key::Soft(7)` | Number keys 4 to 7, under a chip the page shows |
| `Key::Char(c)` | Any key that types, with Shift, Caps and the Sym layer applied |
| `Key::Ctrl(c)`, `Key::Alt(c)` | Ctrl or Alt with a letter or digit |
| `Key::Other(code)` | Anything else, by Linux input code (`gh0st_app::codes`) |

8 opens the app menu and Fn+H opens Help; neither reaches a page. While the
page types, 8 types. Fn+Esc goes back as Esc does, except on a raw-key page,
where it is the Esc key itself.

The OS acts on the brightness, volume, mute and screenshot keys itself. They
still reach the page as `Key::Other`, so a page that shows those values can
read them again.

### Raw keys

A terminal needs every key as it is. A page whose `raw_keys` returns true gets
each press, repeat and release in `Page::raw`, as a `RawKey` with its code, its
text and Shift, Ctrl, Alt and Fn. Plain Esc still goes to `back`, as on every
page; Fn+Esc comes to `raw` as plain Esc, for vim and the like. Every
key but Esc and the modifiers repeats. `RawKey::text` keeps the character
under Ctrl and Alt, so Ctrl+[ can become ESC. Nothing is drawn after a raw key
unless the page calls `cx.redraw()`.

## Commands

A page lists what the user can do once, in `commands`. The SDK builds four
things from that list, so they never disagree:

- **The app menu.** 8 opens it in every app. Rows come in the order the page
  adds them, under the group headings it sets. The SDK adds Help and Close
  app last.
- **Help.** Fn+H opens it: the page's `about` text if it has one, the chips
  on screen, Esc and Enter, every command with keys under the mode it works
  in, then the keys that work everywhere.
  The menu's Help row opens it too, and so does `cx.show_keys()`.
- **The keys.** A command's keys run it while its mode is on. They also pick
  its row while the menu is open.
- **Chips.** `.chip(5, "search", "Find")` puts a chip over key 5 that runs
  the command while it is live. A chip the page sets in `bars` wins.

Three kinds of command:

| Kind | Menu row | Keys and Enter |
|---|---|---|
| `Command::action` | Label, keys on the right. A chevron with `.opens()` | Run it. From the menu, the menu closes first |
| `Command::toggle` | A switch | Flip it in place: the menu stays open |
| `Command::choice` | `‹ value ›` | Left and Right step it in place. Enter steps forward |

The page hears each one in `command(id, step, cx)`. `step` is `Run` for an
action, `Prev` or `Next` for a toggle or a choice, and `Nth(n)` for a key
from a range such as `1-9`. A toggle flips on any step.

```rust
use gh0st_app::prelude::*;

const SIZE: u32 = 1;
const NIGHT: u32 = 2;
const FIND: u32 = 3;
const DOWN: u32 = 4;

const SIZES: &[&str] = &["Small", "Medium", "Large"];

struct Reader {
    size: usize,
    night: bool,
}

impl Page for Reader {
    fn title(&self) -> &str {
        "Reader"
    }

    fn commands(&self, c: &mut Commands) {
        c.group("View");
        // One row, two keys: r steps back, t forward.
        c.add(Command::choice(SIZE, "Text size", SIZES[self.size]).keys("r t"));
        c.add(Command::toggle(NIGHT, "Night", self.night).keys("b"));
        // A menu row, the n key, and a chip over 5.
        c.add(Command::action(FIND, "Find").keys("n").chip(5, "search", "Find").opens());
        // Keys only: in Help, not in the menu.
        c.add(Command::action(DOWN, "Scroll down").keys("x").keys_only().repeats());
    }

    fn command(&mut self, id: u32, step: Step, _: &mut Cx) {
        match (id, step) {
            (SIZE, Step::Prev) => self.size = self.size.saturating_sub(1),
            (SIZE, _) => self.size = (self.size + 1).min(SIZES.len() - 1),
            (NIGHT, _) => self.night = !self.night,
            _ => {}
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.text(SIZES[self.size]);
    }
}
```

### Keys

`keys` takes the keys as the keyboard shows them, split by spaces:

| Spec | Key |
|---|---|
| `n` | N, without Shift |
| `G` | Shift+G |
| `f x z c` | F, X, Z and C, or Fn with them (the arrows) |
| `Esc`, `Enter`, `Tab`, `Space`, `Bksp` | The key of that name |
| `PgUp`, `PgDn`, `Up`, `Del` ... | Keys of the Fn layer. They show as the device's keys: `Fn+L`, `Fn+M`, `Fn+F`, `Fn+Bksp` |
| `Shift+PgUp`, `Ctrl+s`, `Alt+x` | With modifiers |
| `1-9` | Digits 1 to 9: `Step::Nth(0)` to `Nth(8)` |

A choice with two keys steps back on the first and forward on the second.

- **Modes.** `.mode("Scroll mode", on)` names the mode a command's keys work
  in, for Help, and says whether it is on now. Its keys do
  nothing while it is off, but its menu row and the menu's letter for it
  still work.
- **Typing.** While `typing()` is true, letters, digits and symbols type, so
  commands bound to them don't run. Named keys still do.
- **Repeat.** A command runs once per press. With `.repeats()`, a held key
  runs it again and again. Once a press has run a command, that key's
  repeats and release never reach the page, even when the command changed
  the mode.
- **Asking first.** With `.confirm()`, a menu row needs pressing twice; it
  says "Press again" in between. Close app always does.
- **Mode switches.** A toggle with `.closes()` closes the menu once flipped,
  for a mode the user wants to use at once.
- **Hints.** `.hint(word)` names a command for a page that draws its own
  hint bar: `Commands::hints()` lists the live ones.
- **Fn.** Fn with Space, Enter, Tab, R, T, Y, G or V arrives as that key with
  Fn held: `"Fn+Space"`, `"Fn+r"`. The Fn layer changes every other key, and
  no program reads Fn, so these are free for an app's own commands even
  while it types.
- **Disabled.** `.enabled(false)` dims the row and its keys do nothing.
- **Raw keys.** Commands come first for raw-key pages too: `raw` gets only
  the keys no live command took.
- **Ids.** Keep yours below `0xffff_fff0`. The SDK's rows use the top ones.

8 and Fn+H never reach a page. `design.md` has the rules for the menu and
Help.

## A grid of tiles

`Tiles` keeps a grid's selection and scroll and lays its tiles out; the page
draws each one. Bind the arrows as commands and step it; draw the selected
tile's ring with `widgets::tile_focus`, and the gap between tiles is
`theme().m.tile_gap`. The camera's Photos is one.

```rust
use gh0st_app::Tiles;
use gh0st_app::gfx::Rect;

let mut grid = Tiles::new(5, 3); // five across, three rows on screen
grid.set_len(40);
grid.step(0, 3); // down three rows: it scrolls
let (first, shown) = grid.window(); // for a scrollbar
for (i, r) in grid.layout(Rect::new(0, 21, 320, 129), (53, 40), 3) {
    let _ = (i, r); // draw tile i in r
}
assert_eq!((grid.at(), first, shown), (15, 5, 15));
```

## Chips

The screen sits right above the number keys. A chip over 5, 6 or 7 runs when
its key is pressed, and arrives as `Key::Soft(n)`. Key 4 is Back, as in the
template, unless the page puts a chip there too; Esc is still Back. A chip
with no word shows its icon, and Help names it by the command it runs.

```rust
use gh0st_app::prelude::*;

struct Counter {
    n: i32,
}

impl Page for Counter {
    fn title(&self) -> &str {
        "Counter"
    }

    fn bars(&self) -> Bars {
        Bars::top()
            .chip(5, Chip::new("minus", "Less"))
            .chip(6, Chip::new("rotate-cw", "Reset").danger())
            .chip(7, Chip::new("plus", "More"))
    }

    fn key(&mut self, key: Key, _: &mut Cx) {
        match key {
            Key::Soft(5) => self.n -= 1,
            Key::Soft(6) => self.n = 0,
            Key::Soft(7) => self.n += 1,
            _ => {}
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.big(&self.n.to_string());
    }
}
```

- **No chip for Enter or Esc.** Back, Open, Cancel and Save are on those
  keys; the bar is for the actions nobody would guess.
- **4 is Esc** and has no chip. **8 is the menu**, an icon-only chip the SDK
  adds. `.no_menu()` drops it for a page under its own menu.
- **One word**, 54 px at most in Label style: Reload fits, Refresh
  doesn't. A chip shows its word alone; the icon shows only on a chip with
  no word, such as the menu's.
- **Looks.** `.danger()` for taking something away, `.hold()` for loss that
  can't be undone (it runs only after its key is held 0.7 s), `.armed(true)`
  for a press-again step, `.on(true)` for a toggle that is on, `.main()` for
  a main action not on Enter, `.disabled(true)` and `.busy(true)`.
- **Commands.** `Chip::new(..).runs(id)` runs a command instead of sending
  `Key::Soft`; `Command::chip` does the same from the command's side.
- **While typing** digits type, so the bar shows `Bars::hints` instead.
- Icons are the Lucide names gh0st has built in (`assets/icons`).

## Lists

Most settings pages are a list. Keep a `List` in the page for the selection,
and build the rows in `draw`:

```rust
use gh0st_app::prelude::*;

struct Prefs {
    list: List,
    sounds: bool,
}

impl Page for Prefs {
    fn title(&self) -> &str {
        "Preferences"
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match self.list.key(key) {
            Some(Action::Open(0) | Action::Change(0, _)) => self.sounds = !self.sounds,
            Some(Action::Open(1)) => cx.toast("Version 1.0"),
            _ => {}
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.list(
            &self.list,
            &[
                Row::toggle("Key sounds", self.sounds).icon("volume-2"),
                Row::link("About").icon("info"),
            ],
        );
    }
}
```

`list.key` moves the selection on Up and Down. It returns `Action::Open(i)`
for Enter and `Action::Change(i, -1 or 1)` for Left and Right. The list
scrolls to keep the selection in view.

| Row | Right side |
|---|---|
| `Row::link(label)` | A chevron. `.value("80%")` adds the current value |
| `Row::toggle(label, on)` | A switch |
| `Row::choice(label, value)` | The value between arrows, stepped with Left and Right |
| `Row::info(label, value)` | A value, read only |
| `Row::level(label, percent)` | A bar and a percent, stepped with Left and Right |
| `Row::pick(label, picked)` | A check on the one picked, in a list of choices |
| `Row::tag(label, word, tone)` | A count or a word in a tone: "3", "New" |
| `Row::signal(label, level, secure)` | Signal bars and a lock, for a network |
| `Row::text(label)` | Nothing |

`.icon(name)` adds an icon on the left. `.dim()` greys the icon of a row that
isn't available yet. `.disabled("needs a cable")` draws a row that can't be
used now, faint, with why on its right. A long list fills the rest of the page; a short one
leaves room for more below it.

## Text input

A page with a text field returns true from `typing` while the field has focus.
Letters and digits then type instead of moving or running chips, so the action
bar shows the page's hints instead of its chips. Name what Esc and Enter do;
Enter's pill is filled:

```rust
use gh0st_app::prelude::*;

struct Rename {
    name: TextField,
}

impl Page for Rename {
    fn title(&self) -> &str {
        "Rename"
    }

    fn bars(&self) -> Bars {
        Bars::top().hints([("Esc", "cancel"), ("Enter", "save")])
    }

    fn typing(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if self.name.key(key) == Some(Edit::Submit) {
            cx.settings().set_str("rename.name", self.name.text());
            cx.back();
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.note("New name");
        ui.field(&self.name, "Name", true);
    }
}
```

`TextField::masked()` shows dots, for passwords.

### Forms

Two or more fields on a page go in a `Form`. Each has a label above it, Tab
moves between them, and one that is wrong gets a red edge and a line under it
until the user types in it. A form with a single password field uses Tab to
show and hide it instead. `hints` names Tab's job for you:

```rust
use gh0st_app::prelude::*;

struct Account {
    form: Form,
}

impl Page for Account {
    fn title(&self) -> &str {
        "Account"
    }

    fn bars(&self) -> Bars {
        Bars::top().hints(self.form.hints("back", "next"))
    }

    fn typing(&self) -> bool {
        true
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if self.form.key(key) != Some(Edit::Submit) {
            return;
        }
        if self.form.value(1).len() < 4 {
            self.form.error(1, "Too short: 4 or more");
            return;
        }
        let password = self.form.take(1);
        cx.spawn_with_input(&["passwd"], password.as_bytes());
        password.into_bytes().fill(0);
    }

    fn draw(&self, ui: &mut Ui) {
        ui.form(&self.form);
    }
}

let page = Account {
    form: Form::new().text("Your name").password("Password, 4 or more"),
};
# let _ = page;
```

A form zeroes its fields when dropped, and its `Debug` shows labels only.

### Search

A long list gets type-to-search, as home has. Keep a `Search` in the page.
Start it from a Find chip on 6 or the `s` key. While it is active the page
is typing, and `Bars::search` puts the query in the action bar in place of
the chips. Fn+F and Fn+X move, Enter picks, and Esc leaves search before it
leaves the page:

```rust
use gh0st_app::prelude::*;

const NAMES: &[&str] = &["Camera", "Music", "Notes"];

struct Apps {
    list: List,
    search: Search,
}

impl Apps {
    /// The rows that match, best first.
    fn shown(&self) -> Vec<usize> {
        self.search.rank(NAMES.len(), |i| self.search.score(&[NAMES[i]]))
    }
}

impl Page for Apps {
    fn title(&self) -> &str {
        "Apps"
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

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if !self.search.active() && matches!(key, Key::Soft(6) | Key::Char('s')) {
            return self.search.start();
        }
        if self.search.key(key) == Some(Edit::Changed) {
            return self.list.select(0);
        }
        if let Some(Action::Open(i)) = self.list.key(key) {
            cx.toast(NAMES[self.shown()[i]]);
            self.search.cancel();
        }
    }

    fn draw(&self, ui: &mut Ui) {
        let rows: Vec<Row> = self.shown().into_iter().map(|i| Row::text(NAMES[i])).collect();
        ui.list(&self.list, &rows);
    }
}
```

`score` takes the fields to search, best first: a match in a later field
ranks one step lower. It ranks a match at the start of a field first, then
at the start of a word, then anywhere, then the letters in order. Home
ranks the same way (`gh0st_core::search`).

### Long text and app lists

A text longer than the page scrolls. Keep a `Scroll` in the page, pass it
Up and Down, and draw with `ui.scroll_text`. It wraps the text, draws from
the scrolled line and adds a scrollbar.

`ui.apps` draws apps as home's list does: a picture, a name, a line under
it and a tag on the right, such as "Update" or "45%". Its items are
`gfx::widgets::AppItem`s, and a `List` holds the selection:

```rust
use gh0st_app::gfx::widgets::{AppItem, Visual};
use gh0st_app::prelude::*;

struct About {
    scroll: Scroll,
    list: List,
}

impl Page for About {
    fn title(&self) -> &str {
        "About"
    }

    fn key(&mut self, key: Key, _: &mut Cx) {
        if !self.scroll.key(key) {
            self.list.key(key);
        }
    }

    fn draw(&self, ui: &mut Ui) {
        let tint = ui.theme().c.tints[0];
        let app = AppItem {
            label: "Notes",
            note: "Write things down",
            visual: Visual::Letter { ch: 'N', tint },
            pinned: false,
            count: 0,
            tag: Some(("Update", Tone::Info)),
        };
        ui.apps(&self.list, &[app]);
        ui.scroll_text(&self.scroll, "A long text, wrapped to the width.");
    }
}
```

`ui.tiles` draws the same items as home's grid: a badge and a name each, four
to a row, as many rows as fit. A `Grid` holds the selection: Left and Right
step along the rows, Up and Down move a row, and `key` returns the tile Enter
opened. The Store's home is one, with a list in its place when the user
picks List.

## Drawing

Each `Ui` call stacks below the last, inside the page's area, with the OS
margins, fonts and colours:

| Call | Draws |
|---|---|
| `heading`, `text`, `note` | A title line, body text, smaller grey text. Text wraps |
| `section`, `readout` | A small accent label, then label and value pairs |
| `list`, `field` | A list or a text field, as above |
| `big` | Large centred text, for a number or a word |
| `message` | An icon and a line, centred, for an empty or waiting state |
| `notice` | An icon in a tone, a line, a note and the keys that act: empty lists, errors |
| `progress` | A labelled bar, or a moving segment while the end isn't known |
| `qr` | A QR code for a phone camera, with labelled values beside it |
| `space` | A gap, in pixels |

For anything else, draw pixels yourself. `ui.area()` is the page's rect in
screen coordinates, `ui.canvas()` is clipped to it, and `ui.theme()` has the
palette and fonts. `gh0st_app::gfx` has the canvas, colours, fonts and icons:

```rust
use gh0st_app::gfx::Rect;
use gh0st_app::prelude::*;

struct Bar;

impl Page for Bar {
    fn title(&self) -> &str {
        "Level"
    }

    fn draw(&self, ui: &mut Ui) {
        let a = ui.area();
        let accent = ui.theme().c.accent;
        ui.canvas().fill(Rect::new(a.x + 20, a.y + 60, 140, 12), accent);
    }
}
```

Never pick colours or sizes by hand. Take them from the theme by role, so the
app follows the palette the user picks in Settings, custom schemes included:
`theme().c.fg`, `.muted`, `.accent`, `.surface`, `.border`, `.c.soft()`,
`.c.disabled()`, and a tone's colour with `.c.tone(Tone::Warning)`.
`design.md` lists every role. `cx.toast_tone(Tone::Ok, "Saved")` gives a
toast a tone.

`canvas.blit565(x, y, w, h, stride, &px)` copies RGB565 pixels as they are,
clipped, a row at a time: a camera frame or a video. It is the fast path;
`canvas.image` blends and is for icons and pictures with alpha.

## Work in the background

`cx.spawn` runs a program without a shell. Its output reaches `job` a line at
a time, then its exit code. It runs in the app's own event loop, so nothing
blocks and no thread starts:

```rust
use gh0st_app::prelude::*;

struct Volume {
    level: String,
}

impl Page for Volume {
    fn title(&self) -> &str {
        "Volume"
    }

    fn start(&mut self, cx: &mut Cx) {
        cx.spawn(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"]);
    }

    fn job(&mut self, _: Job, event: JobEvent, _: &mut Cx) {
        match event {
            JobEvent::Line(line) => self.level = line,
            JobEvent::Done(code) if code != Some(0) => self.level = "No audio".into(),
            JobEvent::Done(_) => {}
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.text(&self.level);
    }
}
```

Work the user waits for, such as a root helper, gets `cx.busy("Applying…")`
and `cx.idle()` when it ends. After 0.3 s the page dims under a card with
that text, and keys wait until `idle`, so a second press can't start it
again; Esc hides the card. Quicker work never shows it.

`cx.after(ms)` fires `timer` once. `cx.every(ms)` fires it until
`cx.cancel(timer)`. Each firing wakes the CPU, so keep timers for things on
screen. Closing a page stops its timers and kills its jobs.

### Your own fds and children

A job hands out whole lines and closes stdin. When an app needs more, a PTY
or a socket, it joins its own fd to the loop:

- `cx.watch(fd)` returns a `Watch`. `Page::ready` runs when the fd is
  readable or its other end closed. Read until it would block. Make the fd
  non-blocking.
- `cx.write(watch, bytes)` writes without blocking. What doesn't fit waits in
  order and goes out when the fd has room.
- `cx.unwatch(watch)` takes the fd out. The page still owns it and closes it.
- `cx.adopt(pid)` takes a child the page started itself. Its exit comes to
  `Page::exited`. The SDK reaps every child, so never call `waitpid`.

`ready` and `exited` draw only when the page calls `cx.redraw()`, so output
that changes nothing on screen costs no frame. A page that changes faster
than the panel shows returns its least gap from `Page::frame_ms`; the SDK
then draws the latest state at most that often.

```rust,no_run
use gh0st_app::prelude::*;
use std::os::fd::RawFd;

struct Tail {
    fd: RawFd,
    lines: usize,
}

impl Page for Tail {
    fn title(&self) -> &str {
        "Tail"
    }

    fn start(&mut self, cx: &mut Cx) {
        cx.watch(self.fd);
    }

    fn ready(&mut self, _: Watch, cx: &mut Cx) {
        let mut buf = [0u8; 4096];
        // SAFETY: reads into a stack buffer of its own length.
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            self.lines += buf[..n as usize].iter().filter(|&&b| b == b'\n').count();
            cx.redraw();
        }
    }

    fn frame_ms(&self) -> u64 {
        20
    }

    fn draw(&self, ui: &mut Ui) {
        ui.big(&self.lines.to_string());
    }
}
```

### Running behind

When the user allows it (Settings > Apps, or `X-Gh0st-Background=true` in the
`.desktop` file), an app keeps running while another is in front. The shell
tells the SDK over the shell link, and the SDK stops drawing and reading keys
until the app is back. Timers, jobs and watched fds go on. `cx.in_front()`
says which. `cx.notify(text)` reaches the user either way: a toast in the
app, or one the shell draws over whatever is in front.

Every page hears `Page::front(false)` when the app leaves the front, and
`front(true)` when it is back. An app not allowed to run behind is stopped
right after, so `front(false)` is where it lets go of a device, such as a
camera, and finishes a file it was writing.

### Notifications

A notice outlives the toast: it waits in the shell's notifications until the
user clears it, and puts a count on the app's tile. `cx.post(tag, title,
body)` posts one; posting the same `tag` again replaces it, and
`cx.unpost(tag)` takes it back. The shell toasts a new one over whatever is
in front, never while the device is locked.

```rust
use gh0st_app::prelude::*;

struct Recorder;

impl Page for Recorder {
    fn title(&self) -> &str {
        "Recorder"
    }

    fn start(&mut self, cx: &mut Cx) {
        cx.post("saved", "Recording saved", "Memo 12 · 4:31");
    }

    fn draw(&self, ui: &mut Ui) {
        ui.text("Saved");
    }
}
```

[`notify`] is the file format underneath, for a count, a tone, or another
app for Enter to open. `cx.open(name)` asks the shell for another app, by
its desktop file stem or its program: this app pauses and comes back when
that one closes.

### The screen timeout

After `dark_time` with no key, the shell turns the screen off and pauses the
app in front, or sends it behind when it may run in the background. An app
the user watches without typing, a player or a clock, calls
`cx.keep_awake(true)` and the screen stays on while it is in front.
`cx.keep_awake(false)` lets it go dark again. Ask only while it matters:
the screen is the biggest drain after Wi-Fi.

## Settings and files

`cx.settings()` reads and writes `~/.config/cardputerzero/config.json`. The
shell and the stock apps share this file. Each write re-reads it first, so
changes others made meanwhile stay. Start your keys with your app's name and a
dot (`prefs.sounds`).

Some keys belong to the OS. Writing `gh0st.palette` re-themes the app at
once, and the shell when the app exits. `brightness` and `dark_time` hold the
screen's brightness and timeout.

`cx.data_dir()` is a directory for the app's own files,
`~/.local/share/<app id>`.

Passwords and tokens go in the keyring, never in a file. Use the standard
Secret Service API (libsecret, python `keyring`, or `gh0st_dbus` from Rust):
the keyring encrypts them, and only your app reads its own without asking
the user (`docs/keyring.md`). Notifications go through
`org.freedesktop.Notifications` or a notice file (`docs/apps.md`).

## The device

`gh0st_app::system` reads the hardware from sysfs and procfs. None of it needs
root:

| Call | Returns |
|---|---|
| `brightness()`, `set_brightness(percent)` | The screen, 0 to 100 |
| `battery()` | Charge, charging, mAh, and `health()` against the design charge |
| `memory()`, `storage(path)` | Used and total, in MB; `storage` for the filesystem holding `path` |
| `kernel()`, `version()`, `build()` | The kernel release, the gh0st version, and the image build's time and commit |

Wi-Fi, Bluetooth, time and the other root jobs go through the root helper,
`gh0st-helper`. Its sensitive verbs ask for the user's password on stdin, so an
app can't flip them unseen.

## Steps

A run of pages the user walks in order, as first-boot setup is, goes in
`Steps`. It is a page that opens one step at a time above itself, so each step
is an ordinary page and its timers, jobs and dialogs end when it closes. A
step calls `cx.done()` to go on; Esc goes to the step before.

Each step is made as it opens, from a maker that gets its label for the top
bar. A maker that returns `None` is skipped, in whichever direction the user
is going:

```rust
use gh0st_app::prelude::*;

struct Welcome(String);

impl Page for Welcome {
    fn title(&self) -> &str {
        "Welcome"
    }

    // "Setup 1 of 3", after the title.
    fn context(&self) -> Option<&str> {
        Some(&self.0)
    }

    fn draw(&self, ui: &mut Ui) {
        ui.text("Press Enter to start.");
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if key == Key::Enter {
            cx.done();
        }
    }
}

let clock_synced = true;
let setup = Steps::new("Setup")
    .step(|label| Welcome(label.into()))
    .optional(move |label| (!clock_synced).then(|| Welcome(label.into())))
    .step(|label| Welcome(label.into()))
    .on_step(|i| eprintln!("now on step {i}"));
# let _ = setup;
```

`part("Tour")` starts a part numbered on its own: "Tour 2 of 7", then
"Setup 1 of 8" after `part("Setup")`. A part named "" gives empty labels, for
a welcome screen.

Going back makes a step again, so keep what the user chose outside the
steps, in state they share. `Steps::from(i)` opens at step `i`, to resume
after a power cut; `on_step` hears each step as it opens, to save where the
user is. A step that runs a job calls `done` only after the job's `Done`:
closing the step stops its jobs.

### An app that can't close

First-boot setup must be finished. `run_with` takes `Options`:

```rust,no_run
use gh0st_app::{Options, Steps, run_with};

run_with(Steps::new("Setup"), Options { closable: false });
```

Esc held on the first page then does nothing, the menu has no Close app, and
the last page never closes. `cx.quit()` and SIGTERM still end the app, so the
last step quits once its work is done.

## Tests

`gh0st_app::testing::Driver` runs a page without a panel. Feed it keys and
check the frame:

```rust
use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;

struct Count(u32);

impl Page for Count {
    fn title(&self) -> &str {
        "Count"
    }

    fn key(&mut self, key: Key, _: &mut Cx) {
        if key == Key::Enter {
            self.0 += 1;
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.big(&self.0.to_string());
    }
}

let mut d = Driver::new(Count(0));
d.keys("enter\nenter");
assert_eq!(d.depth(), 1);
// The first page closes only on a held Esc.
d.keys("esc");
assert_eq!(d.toast(), Some("Hold to exit"));
d.keys("down esc").wait(800);
assert!(d.quit());
```

Key names follow the sim's script format (`gh0st_core::script`): `enter`,
`esc`, `x`, `fn+z`, `6`, `type hello`, `wait 500`. In scripts, `down esc`
holds a key and `up esc` releases it, so use `fn+f` and `fn+x`, or bare `f`
and `x`, for the arrows.

`d.jobs(ms)` runs the real loop until every job the page spawned has ended,
or `ms` of wall time has passed: lines and exit codes arrive as on the
device. `d.pump(ms)` does the same for watched fds and adopted children.
`Driver::with(page, opts)` runs a page as `run_with` would, and
`d.palette("Nord")` shows it in another palette.

## Rules

- Draw only in your area. The OS owns the bar rows.
- Start no threads and keep no idle timers. The SDK's loop sleeps until a key,
  a job or a timer.
- Open no network connection at start. Wait until the user asks for something
  that needs one.
- Never grab the keyboard (`EVIOCGRAB`). The OS reads it too, for Esc, the
  global keys and the screen timeout.
- Exit quickly on SIGTERM. `stop` runs first.
