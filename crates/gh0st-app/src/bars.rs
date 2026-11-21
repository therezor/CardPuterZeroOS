//! The OS bars a page asks for. The page draws in what is left.

use gh0st_ui::{Rect, Theme};

/// One slot of the soft-key bar, over a number key 4 to 8.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SoftKey {
    pub(crate) icon: String,
    pub(crate) label: String,
    pub(crate) lit: bool,
}

impl SoftKey {
    /// An icon and a label. With an icon, about five letters fit.
    pub fn new(icon: &str, label: &str) -> SoftKey {
        SoftKey {
            icon: icon.into(),
            label: label.into(),
            lit: false,
        }
    }

    /// An icon alone. Icons are the Lucide names gh0st has built in.
    pub fn icon(icon: &str) -> SoftKey {
        SoftKey::new(icon, "")
    }

    /// A label alone. About eight letters fit.
    pub fn label(label: &str) -> SoftKey {
        SoftKey::new("", label)
    }

    /// An empty slot. Its number key types its digit.
    pub fn none() -> SoftKey {
        SoftKey::default()
    }

    /// Draws the slot lit, for a toggle that is on.
    pub fn lit(mut self, on: bool) -> SoftKey {
        self.lit = on;
        self
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.icon.is_empty() && self.label.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Bottom {
    None,
    Hints(Vec<(String, String)>),
    /// Hints from the page's live commands that have one.
    Commands,
    Soft(Box<[SoftKey; 5]>),
}

/// Which OS bars a page shows. The top bar has the title, network, battery
/// and clock. The bottom bar has key hints or soft keys. Every bar left out
/// gives its rows to the page.
///
/// ```
/// use gh0st_app::{Bars, SoftKey};
///
/// let list = Bars::top().hints([("Enter", "open"), ("Esc", "back")]);
/// let camera = Bars::none().softkeys([
///     SoftKey::icon("arrow-left"),
///     SoftKey::icon("zoom-out"),
///     SoftKey::icon("camera"),
///     SoftKey::icon("zoom-in"),
///     SoftKey::icon("images"),
/// ]);
/// let game = Bars::none();
/// # let _ = (list, camera, game);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bars {
    pub(crate) top: bool,
    pub(crate) bottom: Bottom,
}

impl Bars {
    /// No bars: the page has the whole 320x170 screen.
    pub fn none() -> Bars {
        Bars {
            top: false,
            bottom: Bottom::None,
        }
    }

    /// The top bar only. This is the default.
    pub fn top() -> Bars {
        Bars {
            top: true,
            bottom: Bottom::None,
        }
    }

    /// Adds a bottom bar of key hints, such as `("Enter", "open")`.
    pub fn hints<'a>(mut self, hints: impl IntoIterator<Item = (&'a str, &'a str)>) -> Bars {
        self.bottom = Bottom::Hints(
            hints
                .into_iter()
                .map(|(k, l)| (k.to_string(), l.to_string()))
                .collect(),
        );
        self
    }

    /// Adds a bottom bar of key hints taken from the page's commands: each
    /// live command with a [`hint`](crate::Command::hint), in order. The
    /// hints follow the page's mode by themselves.
    pub fn command_hints(mut self) -> Bars {
        self.bottom = Bottom::Commands;
        self
    }

    /// Adds a bottom bar of soft keys, one per number key 4 to 8.
    pub fn softkeys(mut self, keys: [SoftKey; 5]) -> Bars {
        self.bottom = Bottom::Soft(Box::new(keys));
        self
    }

    /// Where the page draws: the screen minus the bars.
    pub fn content(&self, t: &Theme) -> Rect {
        let top = if self.top { t.m.bar_h } else { 0 };
        let bottom = if self.bottom == Bottom::None {
            0
        } else {
            t.m.hint_h
        };
        Rect::new(0, top, t.m.width, t.m.height - top - bottom)
    }

    /// Which soft-key slots show, for keys 4 to 8.
    pub(crate) fn soft_slots(&self) -> [bool; 5] {
        match &self.bottom {
            Bottom::Soft(keys) => std::array::from_fn(|i| !keys[i].is_empty()),
            _ => [false; 5],
        }
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
        assert_eq!(Bars::top().content(&t).h, h - t.m.bar_h);
        let both = Bars::top().hints([("Esc", "back")]);
        assert_eq!(
            both.content(&t),
            Rect::new(0, t.m.bar_h, 320, h - t.m.bar_h - t.m.hint_h)
        );
        let soft = Bars::none().softkeys(Default::default());
        assert_eq!(soft.content(&t).y, 0);
        assert_eq!(soft.content(&t).h, h - t.m.hint_h);
    }

    #[test]
    fn empty_slots_leave_their_digits_alone() {
        let b = Bars::top().softkeys([
            SoftKey::label("Back"),
            SoftKey::none(),
            SoftKey::icon("camera"),
            SoftKey::none(),
            SoftKey::none(),
        ]);
        assert_eq!(b.soft_slots(), [true, false, true, false, false]);
    }
}
