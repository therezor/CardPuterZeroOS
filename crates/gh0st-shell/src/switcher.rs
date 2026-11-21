//! The app switcher: Home, then every open app in the order they opened, as cards across the body. No snapshots: a card is the app's
//! badge, with a pause mark while paused, its name, and its memory, plus CPU
//! for an app running behind. Under the cards, the whole system's CPU and
//! memory as two meters.
//! The bar's dock is the same list in the same order, and its fill follows
//! the cursor.

use crate::home::{self, Icon, Size};
use crate::motion::{self, lerp, lerp_rect};
use gh0st_core::usage::{AppUsage, SystemUsage};
use gh0st_ui::Canvas;
use gh0st_ui::geom::Rect;
use gh0st_ui::theme::Theme;
use gh0st_ui::widgets::{self, Card as CardView, Status, Visual};

const GLIDE_MS: u64 = 80;
/// Room under the cards for the meters.
const STRIP_H: i32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Paused,
    /// Allowed to keep running in the background.
    Running,
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
    Glide {
        from: Rect,
    },
    /// The strip moves a card; cards start `from_px` right of where they end.
    Scroll {
        from_px: i32,
        from: Rect,
    },
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
        let area = widgets::grid(t);
        let gap = 2 * t.m.gap;
        let per = t.m.cards;
        let card_w = (area.w - (per - 1) * gap) / per;
        let x = area.x + (area.w - (per * card_w + (per - 1) * gap)) / 2;
        Geo {
            area: Rect::new(x, area.y + 1, area.w, area.h),
            card_w,
            card_h: area.h - STRIP_H,
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
    /// The stats line per app id, formatted once per sample.
    stats: Vec<(String, String)>,
    meters: Option<Meters>,
}

/// The system's figures under the cards.
#[derive(Clone)]
struct Meters {
    /// Text and share. Memory shows at once; CPU needs a second sample.
    cpu: Option<(String, f32)>,
    ram: String,
    ram_share: f32,
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
            stats: Vec::new(),
            meters: None,
        }
    }

    /// New figures. Leaves the cursor and any motion alone. A paused app
    /// shows memory only: it uses no CPU. A figure that needs CPU waits for
    /// it, so a line appears whole. True if any text changed.
    pub fn set_usage(&mut self, apps: &[(&str, AppUsage)], system: SystemUsage) -> bool {
        let stats: Vec<(String, String)> = apps
            .iter()
            .filter_map(|(id, u)| {
                let card = self.apps.iter().find(|c| c.id == *id)?;
                let text = match (card.state, u.cpu) {
                    (State::Paused, _) => mb(u.mem_kb),
                    // ASCII only: the figures are mono, and its ASCII glyphs are
                    // indexed directly, so no other page of the table is read.
                    (State::Running, Some(cpu)) => format!("{}  {}", mb(u.mem_kb), pct(cpu)),
                    // No CPU yet after a gap: keep the last line.
                    _ => self.stats.iter().find(|(i, _)| i == id)?.1.clone(),
                };
                Some((id.to_string(), text))
            })
            .collect();
        let cpu = match system.cpu {
            Some(cpu) => Some((pct(cpu), cpu / 100.0)),
            // No CPU after a gap: keep the last figure.
            None => self.meters.as_ref().and_then(|m| m.cpu.clone()),
        };
        let meters = Some(Meters {
            cpu,
            ram: format!(
                "{}/{} MB",
                mb_round(system.used_kb),
                mb_round(system.total_kb)
            ),
            ram_share: system.used_kb as f32 / system.total_kb.max(1) as f32,
        });
        // Only the text decides a redraw: a bar moves less than a pixel
        // between two values that print the same.
        let text = |m: &Option<Meters>| {
            m.as_ref()
                .map(|m| (m.cpu.as_ref().map(|c| c.0.clone()), m.ram.clone()))
        };
        let changed = stats != self.stats || text(&meters) != text(&self.meters);
        self.stats = stats;
        self.meters = meters;
        changed
    }

    /// Drops every figure, for a switcher about to open.
    pub fn clear_usage(&mut self) {
        self.stats.clear();
        self.meters = None;
    }

    fn stats_of(&self, id: &str) -> &str {
        self.stats
            .iter()
            .find(|(i, _)| i == id)
            .map_or("", |(_, s)| s.as_str())
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
            let card = match index.checked_sub(1).map(|i| &self.apps[i]) {
                None => CardView {
                    label: "Home",
                    note: "",
                    // An empty line, so every badge sits on one row.
                    stats: Some(""),
                    paused: false,
                    visual: Visual::System {
                        icon: widgets::LOGO,
                    },
                },
                Some(app) => CardView {
                    label: &app.name,
                    note: match app.state {
                        State::Closing => "Closing\u{2026}",
                        _ => "",
                    },
                    stats: Some(self.stats_of(&app.id)),
                    paused: app.state == State::Paused,
                    visual: home::visual(t, &app.icon, &app.name, Size::Large),
                },
            };
            widgets::card_content(c, t, r, &card, index == self.selected);
        }
        widgets::selection_ring(c, t, frame);
        c.set_clip(old);
        self.meters(c, t);
        // 5 closes the app in front of you, at once; 7 closes them all,
        // after asking. Apart, so neither is pressed for the other.
        use widgets::{Chip, Look};
        let none = Chip::default();
        let danger = |icon, word| Chip {
            icon,
            word,
            look: Look::Danger,
        };
        let close = if self.selected > 0 {
            danger("circle-x", "Close")
        } else {
            none
        };
        let all = if self.apps.is_empty() {
            none
        } else {
            danger("copy-x", "All")
        };
        widgets::action_bar(c, t, &[none, close, none, all, none]);
    }

    /// CPU and memory, side by side across the cards' width. The top bar's
    /// dock marks the selected app, so no dots are needed here.
    fn meters(&self, c: &mut Canvas, t: &Theme) {
        let Some(m) = &self.meters else {
            return;
        };
        let (first, last) = (self.geo.card(0), self.geo.card(self.geo.per as i32 - 1));
        let y = first.bottom() + 3;
        let h = self.geo.area.bottom() - 1 - y;
        let w = (last.right() - first.x - self.geo.gap) / 2;
        let left = Rect::new(first.x, y, w, h);
        let right = Rect::new(last.right() - w, y, w, h);
        // Before its first figure, the CPU meter is an empty track.
        let (cpu, share) = m.cpu.as_ref().map_or(("", 0.0), |(s, f)| (s.as_str(), *f));
        widgets::meter(c, t, left, "CPU", cpu, share);
        widgets::meter(c, t, right, "RAM", &m.ram, m.ram_share);
    }
}

fn mb_round(kb: u64) -> u64 {
    (kb + 512) / 1024
}

fn mb(kb: u64) -> String {
    if kb < 10 * 1024 {
        format!("{:.1} MB", kb as f32 / 1024.0)
    } else {
        format!("{} MB", mb_round(kb))
    }
}

/// Whole percents: tenths would change, and redraw, on every sample.
fn pct(p: f32) -> String {
    format!("{p:.0}%")
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
    fn figures_show_memory_and_cpu_only_for_an_app_running_behind() {
        let mut s = switcher(&["a", "b", "c"]);
        let mut apps: Vec<Card> = s.apps().to_vec();
        apps[1].state = State::Running;
        apps[2].state = State::Closing;
        s.set_apps(apps);
        let mem = |mem_kb, cpu| AppUsage { mem_kb, cpu };
        let system = |cpu| SystemUsage {
            used_kb: 200 * 1024,
            total_kb: 352 * 1024,
            cpu,
        };
        let first = [("a", mem(40_000, None)), ("b", mem(5_000, None))];
        assert!(s.set_usage(&first, system(None)));
        // A running app and the system wait for their CPU.
        assert_eq!(s.stats_of("a"), "39 MB");
        assert_eq!(s.stats_of("b"), "");
        let m = s.meters.as_ref().unwrap();
        assert_eq!((m.cpu.is_none(), m.ram.as_str()), (true, "200/352 MB"));
        let next = [("a", mem(40_000, Some(0.0))), ("b", mem(5_000, Some(12.4)))];
        assert!(s.set_usage(&next, system(Some(6.6))));
        assert_eq!(s.stats_of("a"), "39 MB");
        assert_eq!(s.stats_of("b"), "4.9 MB  12%");
        let m = s.meters.as_ref().unwrap();
        assert_eq!(m.cpu.as_ref().unwrap().0, "7%");
        // Same text, no redraw. A gap with no CPU keeps the last lines.
        assert!(!s.set_usage(&next, system(Some(6.7))));
        assert!(!s.set_usage(&first, system(None)));
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
