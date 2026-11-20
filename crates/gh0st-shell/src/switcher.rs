//! The app switcher: Home, then every open app in launch order, as cards
//! across the body. No snapshots: a card is the app's badge, name and state.
//! The bar's dock is the same list in the same order, and its fill follows
//! the cursor.

use crate::home::{self, Icon, Size};
use crate::motion::{self, lerp, lerp_rect};
use gh0st_ui::Canvas;
use gh0st_ui::geom::Rect;
use gh0st_ui::theme::Theme;
use gh0st_ui::widgets::{self, Card as CardView, Hint, Status, Visual};

const GLIDE_MS: u64 = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Paused,
    /// Sent SIGTERM; waiting for its group to go.
    Closing,
}

#[derive(Clone)]
pub struct Card {
    /// The app's `.desktop` file name.
    pub id: String,
    pub name: String,
    pub icon: Icon,
    pub state: State,
}

#[derive(Clone, Copy, Debug)]
enum Motion {
    Glide { from: Rect },
    /// The strip moves a card; cards start `from_px` right of where they end.
    Scroll { from_px: i32, from: Rect },
}

type Anim = motion::Anim<Motion>;

#[derive(Clone, Copy, Debug)]
struct Geo {
    area: Rect,
    card_w: i32,
    card_h: i32,
    gap: i32,
    per: usize,
}

impl Geo {
    fn new(t: &Theme) -> Geo {
        let area = widgets::content(t);
        let gap = 2 * t.m.gap;
        let per = t.m.cards;
        let card_w = (area.w - (per - 1) * gap) / per;
        let x = area.x + (area.w - (per * card_w + (per - 1) * gap)) / 2;
        Geo {
            area: Rect::new(x, area.y + 1, area.w, area.h),
            card_w,
            card_h: area.h - 16,
            gap,
            per: per as usize,
        }
    }

    fn card(&self, slot: i32) -> Rect {
        Rect::new(
            self.area.x + slot * (self.card_w + self.gap),
            self.area.y,
            self.card_w,
            self.card_h,
        )
    }
}

pub struct Switcher {
    /// Open apps. Card 0, Home, is not stored.
    apps: Vec<Card>,
    /// 0 is Home, n the nth app.
    selected: usize,
    /// First card on screen.
    top: usize,
    geo: Geo,
    animations: bool,
    anim: Option<Anim>,
}

impl Switcher {
    pub fn new(t: &Theme) -> Switcher {
        Switcher {
            apps: Vec::new(),
            selected: 0,
            top: 0,
            geo: Geo::new(t),
            animations: true,
            anim: None,
        }
    }

    pub fn set_animations(&mut self, on: bool) {
        self.animations = on;
        if !on {
            self.anim = None;
        }
    }

    /// New app list. The cursor stays on the same app, or on Home if it went.
    pub fn set_apps(&mut self, apps: Vec<Card>) {
        let keep = self.selected_app().map(|c| c.id.clone());
        self.apps = apps;
        self.selected = keep
            .and_then(|id| self.apps.iter().position(|c| c.id == id))
            .map_or(0, |i| i + 1);
        self.anim = None;
        self.keep_visible();
    }

    pub fn apps(&self) -> &[Card] {
        &self.apps
    }

    /// Puts the cursor on card `n` (0 is Home), with no motion.
    pub fn select(&mut self, n: usize) {
        self.selected = n.min(self.apps.len());
        self.anim = None;
        self.keep_visible();
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The app under the cursor, or `None` on Home.
    pub fn selected_app(&self) -> Option<&Card> {
        self.selected.checked_sub(1).and_then(|i| self.apps.get(i))
    }

    fn count(&self) -> usize {
        self.apps.len() + 1
    }

    fn keep_visible(&mut self) {
        let per = self.geo.per;
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + per {
            self.top = self.selected + 1 - per;
        }
        self.top = self.top.min(self.count().saturating_sub(per));
    }

    fn frame(&self, index: usize, top: usize) -> Rect {
        self.geo.card(index as i32 - top as i32)
    }

    /// Left or right one card, no wrap.
    pub fn move_by(&mut self, dx: i32, now: u64) {
        let (old, old_top) = (self.selected, self.top);
        let from = self.current_frame(now);
        let last = self.count() - 1;
        self.selected = (self.selected as i64 + dx as i64).clamp(0, last as i64) as usize;
        self.keep_visible();
        if !self.animations || self.selected == old {
            return;
        }
        let from = from.unwrap_or_else(|| self.frame(old, old_top));
        let motion = if self.top == old_top {
            Motion::Glide { from }
        } else {
            Motion::Scroll {
                from_px: (self.top as i32 - old_top as i32) * (self.geo.card_w + self.geo.gap),
                from,
            }
        };
        self.anim = Some(Anim {
            start: now,
            ms: GLIDE_MS,
            motion,
        });
    }

    fn current_frame(&self, now: u64) -> Option<Rect> {
        let a = self.anim?;
        let to = self.frame(self.selected, self.top);
        match a.motion {
            Motion::Glide { from } | Motion::Scroll { from, .. } => {
                Some(lerp_rect(from, to, a.progress(now)))
            }
        }
    }

    pub fn next_frame(&self, now: u64) -> Option<u64> {
        self.anim.map(|a| a.next_frame(now))
    }

    pub fn settle(&mut self, now: u64) {
        if self.anim.is_some_and(|a| a.done(now)) {
            self.anim = None;
        }
    }

    /// The bar's dock for this list, in the same order as the cards.
    pub fn dock<'a>(&'a self, t: &Theme) -> Vec<Visual<'a>> {
        self.apps
            .iter()
            .map(|c| home::visual(t, &c.icon, &c.name, Size::Tiny))
            .collect()
    }

    pub fn draw(&self, c: &mut Canvas, t: &Theme, status: &Status, now: u64) {
        c.fill(widgets::body(t), t.c.bg);
        widgets::status_bar(c, t, status);
        let to = self.frame(self.selected, self.top);
        let (frame, offset) = match self.anim {
            Some(a) => match a.motion {
                Motion::Glide { from } => (lerp_rect(from, to, a.progress(now)), 0),
                Motion::Scroll { from_px, from } => {
                    let p = a.progress(now);
                    (lerp_rect(from, to, p), lerp(from_px, 0, p))
                }
            },
            None => (to, 0),
        };
        let old = c.set_clip(widgets::body(t));
        widgets::selection(c, t, frame);
        // One card past each edge, so a scroll never shows a gap.
        for slot in -1..=self.geo.per as i32 {
            let index = self.top as i32 + slot;
            if index < 0 || index as usize >= self.count() {
                continue;
            }
            let r = self.geo.card(slot);
            let r = Rect::new(r.x + offset, r.y, r.w, r.h);
            let index = index as usize;
            let note;
            let card = match index.checked_sub(1).map(|i| &self.apps[i]) {
                None => CardView {
                    label: "Home",
                    note: match self.apps.len() {
                        0 => "Nothing open",
                        1 => "1 app open",
                        n => {
                            note = format!("{n} apps open");
                            &note
                        }
                    },
                    visual: Visual::Badge {
                        icon: widgets::SKULL,
                        tint: t.c.accent,
                    },
                },
                Some(app) => CardView {
                    label: &app.name,
                    note: match app.state {
                        State::Paused => "Paused",
                        State::Closing => "Closing\u{2026}",
                    },
                    visual: home::visual(t, &app.icon, &app.name, Size::Large),
                },
            };
            widgets::card_content(c, t, r, &card, index == self.selected);
        }
        widgets::selection_ring(c, t, frame);
        c.set_clip(old);
        self.dots(c, t);
        let mut hints = vec![
            Hint {
                key: "Z C",
                label: "move",
            },
            Hint {
                key: "Enter",
                label: "open",
            },
        ];
        if self.selected > 0 {
            hints.push(Hint {
                key: "Del",
                label: "close",
            });
        }
        hints.push(Hint {
            key: "Esc",
            label: "back",
        });
        widgets::hint_bar(c, t, &hints, None);
    }

    /// One dot per card, centred under the strip, when they do not all fit.
    fn dots(&self, c: &mut Canvas, t: &Theme) {
        let n = self.count();
        if n <= self.geo.per {
            return;
        }
        let w = (n as i32 - 1) * 8;
        let x0 = self.geo.area.x + (self.geo.area.w - w) / 2;
        let cy = self.geo.area.y + self.geo.card_h + 8;
        for i in 0..n {
            let col = if i == self.selected {
                t.c.accent
            } else {
                t.c.border
            };
            c.circle((x0 + i as i32 * 8) as f32, cy as f32, 2.0, col);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str) -> Card {
        Card {
            id: id.into(),
            name: id.into(),
            icon: Icon::Letter { tint: 0 },
            state: State::Paused,
        }
    }

    fn switcher(ids: &[&str]) -> Switcher {
        let mut s = Switcher::new(&Theme::default());
        s.set_apps(ids.iter().map(|i| card(i)).collect());
        s
    }

    #[test]
    fn home_is_card_zero_and_moves_stop_at_the_ends() {
        let mut s = switcher(&["a", "b"]);
        assert_eq!(s.selected(), 0);
        assert!(s.selected_app().is_none());
        s.move_by(-1, 0);
        assert_eq!(s.selected(), 0);
        s.move_by(1, 0);
        s.move_by(1, 0);
        s.move_by(1, 0);
        assert_eq!(s.selected_app().unwrap().id, "b");
    }

    #[test]
    fn the_strip_scrolls_once_the_cursor_passes_the_last_card_on_screen() {
        let mut s = switcher(&["a", "b", "c", "d"]);
        s.select(2);
        assert_eq!(s.top, 0);
        s.move_by(1, 0);
        assert_eq!((s.selected, s.top), (3, 1));
        assert!(matches!(s.anim.unwrap().motion, Motion::Scroll { .. }));
    }

    #[test]
    fn a_new_list_keeps_the_cursor_on_its_app_or_falls_back_to_home() {
        let mut s = switcher(&["a", "b", "c"]);
        s.select(2);
        s.set_apps(vec![card("x"), card("b")]);
        assert_eq!(s.selected_app().unwrap().id, "b");
        s.set_apps(vec![card("x")]);
        assert_eq!(s.selected(), 0);
    }
}
