# Writing a gh0st app

`gh0st-app` is the SDK for CardputerZero apps written in Rust. An app is a
stack of pages. Each page says which OS bars it wants, draws itself, and
answers keys. The SDK runs the event loop, maps the keyboard, draws the bars,
and sends only the changed rows to the panel.

Settings (`crates/gh0st-settings`) is built on this SDK, plus `gh0st-core`
for the app list, pins and the backlight. Read it for a full app. Apps in other languages use the contract in `apps.md` directly.

Status: the SDK draws the OS bars inside the app for now. Once the shell link
lands (roadmap A1), the shell draws them instead. Apps don't change.

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
        ui.text("This page has the OS top bar. Press Esc to leave.");
    }
}

fn main() {
    gh0st_app::run(Hello);
}
```

Run it on a Mac or Linux PC. Keys come from stdin, one per line, and each new
frame is saved as a PNG:

```sh
echo esc | cargo run -- --sim frames
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
| `Bars::top()`, the default | 320x152, below the top bar |
| `Bars::top().hints(...)` or `.softkeys(...)` | 320x134, between the bars |
| `Bars::none().softkeys(...)` | 320x152, above the soft keys |
| `Bars::none()` | all 320x170 |

The top bar shows the page title, network, battery and clock. The bottom bar
shows key hints or soft keys. A page can change its bars at any time, for
example to hide them while a video plays:

```rust
use gh0st_app::prelude::*;

struct Player {
    playing: bool,
}

impl Page for Player {
    fn title(&self) -> &str {
        "Player"
    }

    fn bars(&self) -> Bars {
        if self.playing {
            Bars::none()
        } else {
            Bars::top().hints([("Enter", "play"), ("Esc", "back")])
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.message("square-terminal", "Nothing playing");
    }
}
```

## Pages

The page passed to `run` is the app. `cx.push(page)` opens another page on top,
and Esc closes the top page. When the last page closes, the app exits.

A page needs `title` and `draw`. The other methods have defaults:

| Method | Called when |
|---|---|
| `bars` | Before each frame. The default is the top bar alone |
| `key` | A key is pressed. Esc goes to `back` instead |
| `back` | Esc is pressed. Return false to keep the page open |
| `typing` | Before each key. Return true while a text field has focus |
| `start` | The page opens |
| `resume` | The page above it closed |
| `timer` | A timer from `cx.after` or `cx.every` fires |
| `job` | A process from `cx.spawn` prints a line or exits |
| `stop` | The page closes or the app quits. Save state here |

After every event the SDK calls `draw` and compares the frame with the last
one. Only rows that changed go over SPI. A page never asks for a redraw and
never keeps a timer to animate an idle screen.

When the user holds Esc for 3 seconds, the OS sends SIGTERM. The SDK then
calls `stop` on every page, top first, and exits.

## Keys

| Key | Physical key |
|---|---|
| `Key::Up`, `Down`, `Left`, `Right` | Fn+F, X, Z, C. Bare F, X, Z and C too, unless the page is typing |
| `Key::Enter`, `Tab`, `Backspace`, `Delete` | The key of that name |
| `Key::Back` | Esc. It goes to `Page::back`, not `Page::key` |
| `Key::Soft(4)` to `Key::Soft(8)` | Number keys 4 to 8, while their soft-key slot shows |
| `Key::Char(c)` | Any key that types, with Shift, Caps and the Sym layer applied |
| `Key::Ctrl(c)`, `Key::Alt(c)` | Ctrl or Alt with a letter or digit |
| `Key::Other(code)` | Anything else, by Linux input code (`gh0st_app::codes`) |

Fn+Esc and Fn+H never reach a page: they open the app menu.

The OS acts on the brightness, volume, mute and screenshot keys itself. They
still reach the page as `Key::Other`, so a page that shows those values can
read them again.

### Raw keys

A terminal needs every key as it is. A page whose `raw_keys` returns true gets
each press, repeat and release in `Page::raw`, as a `RawKey` with its code, its
text and Shift, Ctrl, Alt and Fn. Esc comes there too, not to `back`. Every
key but Esc and the modifiers repeats. `RawKey::text` keeps the character
under Ctrl and Alt, so Ctrl+[ can become ESC. Nothing is drawn after a raw key
unless the page calls `cx.redraw()`.

## The app menu

Fn+Esc or Fn+H opens the app menu in every app (the rules are in `apps.md`,
The app menu). The page in front lists its items; the SDK draws the list, handles
the keys and adds Close app last. A pick comes back to `menu_pick` with the
item's id after the menu has closed. Pages with raw keys get the menu too:
the SDK takes those keys before `raw` sees them.

```rust
use gh0st_app::prelude::*;

struct Reader {
    large: bool,
}

const LARGER: u32 = 1;
const FIND: u32 = 2;

impl Page for Reader {
    fn title(&self) -> &str {
        "Reader"
    }

    fn menu(&self) -> Vec<MenuItem> {
        vec![
            MenuItem::new(LARGER, "Large text")
                .key('t')
                .value(if self.large { "On" } else { "Off" }),
            MenuItem::new(FIND, "Find").key('n'),
        ]
    }

    fn menu_pick(&mut self, id: u32, _: &mut Cx) {
        if id == LARGER {
            self.large = !self.large;
        }
    }

    fn draw(&self, ui: &mut Ui) {
        ui.text(if self.large { "LARGE" } else { "small" });
    }
}
```

## Soft keys

The screen sits right above the number keys 4 to 8. The soft-key bar puts one
slot over each key:

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
        Bars::none().softkeys([
            SoftKey::new("arrow-left", "Back"),
            SoftKey::label("Less"),
            SoftKey::label("Reset"),
            SoftKey::label("More"),
            SoftKey::none(),
        ])
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        match key {
            Key::Soft(4) => cx.back(),
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

- Slot 4 is back by convention, as in the stock apps.
- With an icon, a label fits about five letters. A label alone fits about
  eight.
- `SoftKey::none()` leaves a slot empty, and its number key types its digit.
- `.lit(true)` marks a toggle that is on, such as a camera grid.
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

    fn bars(&self) -> Bars {
        Bars::top().hints([("Enter", "open"), ("Esc", "back")])
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
| `Row::text(label)` | Nothing |

`.icon(name)` adds an icon on the left. `.dim()` greys the icon of a row that
isn't available yet. A long list fills the rest of the page; a short one
leaves room for more below it.

## Text input

A page with a text field returns true from `typing` while the field has focus.
Letters and digits then type instead of moving or pressing soft keys. Show
hints in that state, not soft keys:

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
        Bars::top().hints([("Enter", "save"), ("Esc", "cancel")])
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

Never pick colours or sizes by hand. Take them from the theme, so the app
follows the palette the user picks in Settings.

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

## The device

`gh0st_app::system` reads the hardware from sysfs and procfs. None of it needs
root:

| Call | Returns |
|---|---|
| `brightness()`, `set_brightness(percent)` | The screen, 0 to 100 |
| `battery()` | Charge, charging, mAh, and `health()` against the design charge |
| `memory()`, `storage()` | Used and total, in MB |
| `kernel()`, `version()` | The kernel release and the gh0st version |

Wi-Fi, Bluetooth, time and the other root jobs come with the root helper
(roadmap M3).

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
d.keys("esc");
assert!(d.quit());
```

Key names follow the sim's script format (`gh0st_core::script`): `enter`,
`esc`, `x`, `fn+z`, `6`, `type hello`, `wait 500`. In scripts, `down esc`
holds a key and `up esc` releases it, so use `fn+f` and `fn+x`, or bare `f`
and `x`, for the arrows.

## Rules

- Draw only in your area. The OS owns the bar rows.
- Start no threads and keep no idle timers. The SDK's loop sleeps until a key,
  a job or a timer.
- Open no network connection at start. Wait until the user asks for something
  that needs one.
- Never grab the keyboard (`EVIOCGRAB`). The OS reads it too, for Esc and the
  global keys.
- Exit quickly on SIGTERM. `stop` runs first.
