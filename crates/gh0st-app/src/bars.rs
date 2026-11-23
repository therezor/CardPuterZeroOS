//! The OS bars a page asks for. The page draws in what is left.

use gh0st_ui::widgets::Look;
use gh0st_ui::{Rect, Theme};

/// An action over one of the number keys 5, 6 or 7: an icon and one short
/// word. Pressing the key runs it. docs/design.md says when to use each
/// look; a chip never repeats Enter or Esc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub(crate) icon: String,
    pub(crate) word: String,
    pub(crate) look: Look,
    /// A command it runs, instead of `Key::Soft`.
    pub(crate) command: Option<u32>,
    /// It runs only after its key is held, filling red: for loss of data.
    pub(crate) hold: bool,
}

impl Chip {
    /// An icon and a word. Five letters or fewer keep the icon on a full
    /// bar. Icons are the Lucide names gh0st has built in.
    pub fn new(icon: &str, word: &str) -> Chip {
        Chip {
            icon: icon.into(),
            word: word.into(),
            look: Look::Normal,
            command: None,
            hold: false,
        }
    }

    /// Runs the command `id` when pressed, as its keys and menu row do.
    pub fn runs(mut self, id: u32) -> Chip {
        self.command = Some(id);
        self
    }

    /// Takes something away: a red edge.
    pub fn danger(mut self) -> Chip {
        self.look = Look::Danger;
        self
    }

    /// Loses data that can't come back: red, and it runs only after its key
    /// is held 0.7 s, filling as it goes.
    pub fn hold(mut self) -> Chip {
        self.look = Look::Danger;
        self.hold = true;
        self
    }

    /// Armed: pressing it again does it. Filled red.
    pub fn armed(mut self, on: bool) -> Chip {
        if on {
            self.look = Look::DangerMain;
        }
        self
    }

    /// The page's main action, when it is not on Enter.
    pub fn main(mut self) -> Chip {
        self.look = Look::Main;
        self
    }

    /// A toggle that is on.
    pub fn on(mut self, on: bool) -> Chip {
        if on {
            self.look = Look::On;
        }
        self
    }

    /// Can't be used now: drawn faint, and its key does nothing.
    pub fn disabled(mut self, off: bool) -> Chip {
        if off {
            self.look = Look::Disabled;
        }
        self
    }

    /// Working on it: "…" after the word, and no second press.
    pub fn busy(mut self, busy: bool) -> Chip {
        if busy {
            self.look = Look::Busy;
        }
        self
    }

    /// Icon, word and look, as the action bar draws them: for mockups.
    pub fn parts(&self) -> (&str, &str, Look) {
        (&self.icon, &self.word, self.look)
    }

    pub(crate) fn runs_key(&self) -> bool {
        !matches!(self.look, Look::Disabled | Look::Busy)
    }
}

/// Which OS bars a page shows. The top bar says where you are and shows the
/// device. The action bar under the screen has the page's chips over keys 5
/// to 7 and the menu on 8; while the page is typing it shows key hints
/// instead. Every bar left out gives its rows to the page.
///
/// ```
/// use gh0st_app::{Bars, Chip};
///
/// let list = Bars::top().chip(5, Chip::new("search", "Find"));
/// let form = Bars::top().hints([("Esc", "cancel"), ("Enter", "save")]);
/// let camera = Bars::actions()
///     .chip(5, Chip::new("zoom-out", "Out"))
///     .chip(6, Chip::new("zoom-in", "In"));
/// let game = Bars::none();
/// # let _ = (list, form, camera, game);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bars {
    pub(crate) top: bool,
    pub(crate) bottom: bool,
    /// Keys 4 to 7. A chip on 4 takes it from Back; Esc stays Back.
    pub(crate) chips: [Option<Chip>; 4],
    /// The menu chip on 8.
    pub(crate) menu: bool,
    /// The hint bar while typing.
    pub(crate) hints: Vec<(String, String)>,
    /// The menu chip alone, in the bottom right corner of a full screen.
    pub(crate) corner: bool,
    /// The action bar lies over the page, which keeps the whole height.
    pub(crate) over: bool,
}

impl Bars {
    /// No bars: the page has the whole 320x170 screen. Esc, Enter and Fn+H
    /// still work; with no chip on 8, Help then 8 opens the menu.
    pub fn none() -> Bars {
        Bars {
            top: false,
            bottom: false,
            chips: [None, None, None, None],
            menu: true,
            hints: Vec::new(),
            corner: false,
            over: false,
        }
    }

    /// The whole screen, as [`none`](Bars::none), but 8 opens the menu and
    /// its chip stands over the right rail's foot (`rail_w` wide). For a
    /// viewfinder or a game that keeps that corner for it.
    pub fn full() -> Bars {
        Bars {
            corner: true,
            ..Bars::none()
        }
    }

    /// The top bar and the action bar, with the menu on 8. The default.
    pub fn top() -> Bars {
        Bars {
            top: true,
            bottom: true,
            ..Bars::none()
        }
    }

    /// The top bar alone: a page that takes every key, as a terminal does.
    pub fn top_only() -> Bars {
        Bars {
            top: true,
            ..Bars::none()
        }
    }

    /// The action bar alone, for a full-screen page with actions.
    pub fn actions() -> Bars {
        Bars {
            bottom: true,
            ..Bars::none()
        }
    }

    /// A chip over key 5, 6 or 7. Key 4 is Back unless a page puts a chip
    /// there, for a page whose Esc is enough: the camera's Photos.
    pub fn chip(mut self, key: u8, chip: Chip) -> Bars {
        if let Some(slot) = (4..=7)
            .contains(&key)
            .then(|| &mut self.chips[(key - 4) as usize])
        {
            *slot = Some(chip);
        }
        self
    }

    /// The action bar over the page instead of below it, on a translucent
    /// strip: the page keeps all 170 rows and must leave the foot's 20
    /// for the chips to read. For a viewfinder.
    pub fn over(mut self) -> Bars {
        self.over = true;
        self
    }

    /// No menu chip on 8, for a page under its own menu.
    pub fn no_menu(mut self) -> Bars {
        self.menu = false;
        self
    }

    /// Key hints shown while the page is typing, when digits type and no chip
    /// can run: `("Esc", "cancel"), ("Enter", "save")`. Enter's pill is
    /// filled. Without these the bar names Esc and Enter plainly. A page
    /// with hints and no chips shows them too: a mode, such as moving a row.
    pub fn hints<'a>(mut self, hints: impl IntoIterator<Item = (&'a str, &'a str)>) -> Bars {
        self.hints = hints
            .into_iter()
            .map(|(k, l)| (k.to_string(), l.to_string()))
            .collect();
        self
    }

    /// Where the page draws: the screen minus the bars.
    pub fn content(&self, t: &Theme) -> Rect {
        let top = if self.top { t.m.bar_h } else { 0 };
        let bottom = if self.bottom && !self.over {
            t.m.hint_h
        } else {
            0
        };
        Rect::new(0, top, t.m.width, t.m.height - top - bottom)
    }
}

impl Default for Bars {
    fn default() -> Bars {
        Bars::top()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_bar_left_out_gives_its_rows_to_the_page() {
        let t = Theme::default();
        let h = t.m.height;
        assert_eq!(Bars::none().content(&t).h, h);
        assert_eq!(Bars::top_only().content(&t).h, h - t.m.bar_h);
        assert_eq!(
            Bars::top().content(&t),
            Rect::new(0, t.m.bar_h, 320, h - t.m.bar_h - t.m.hint_h)
        );
        let actions = Bars::actions();
        assert_eq!(actions.content(&t).y, 0);
        assert_eq!(actions.content(&t).h, h - t.m.hint_h);
    }

    #[test]
    fn a_full_page_keeps_every_row_and_the_menu() {
        let t = Theme::default();
        let b = Bars::full();
        assert_eq!(b.content(&t), Rect::new(0, 0, t.m.width, t.m.height));
        assert!(b.menu && b.corner && !b.bottom);
    }

    #[test]
    fn chips_go_on_four_to_seven_and_eight_stays_the_menu() {
        let b = Bars::top()
            .chip(6, Chip::new("camera", "Shoot"))
            .chip(8, Chip::new("x", "Menu"))
            .chip(3, Chip::new("x", "Three"));
        let words: Vec<_> = b
            .chips
            .iter()
            .map(|c| c.as_ref().map(|c| c.word.as_str()))
            .collect();
        assert_eq!(words, [None, None, Some("Shoot"), None]);
        let b = Bars::top().chip(4, Chip::new("images", "Photos"));
        assert_eq!(b.chips[0].as_ref().map(|c| c.word.as_str()), Some("Photos"));
    }
}
