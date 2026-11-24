//! Pairing a new host. The keyboard is discoverable only while this
//! screen is open, and says yes to pairing only here, after the user has
//! seen the code.

use gh0st_app::gfx::Rect;
use gh0st_app::gfx::widgets::{self, Hint};
use gh0st_app::prelude::*;

use crate::app::{Nav, Screen, Shared, address_of};
use crate::host::System;
use crate::link::{Ask, Event};
use crate::pages::profile::Profile;
use crate::store::Host;
use crate::words;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    /// The advert ran out with nobody paired.
    Ended,
    /// The host gave up.
    Cancelled,
}

pub struct Pair {
    phase: Phase,
    /// The device asking, by BlueZ path.
    device: String,
    code: TextField,
    /// The host asked for its code typed here: digits type.
    entering: bool,
}

impl Pair {
    pub fn new() -> Pair {
        Pair {
            phase: Phase::Waiting,
            device: String::new(),
            code: TextField::new(),
            entering: false,
        }
    }

    fn asking(s: &Shared) -> Option<Ask> {
        s.bt.as_ref()?.asking()
    }

    fn host_name(&self, s: &Shared) -> String {
        s.device_at(&self.device)
            .map(|d| d.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| words::THE_HOST.into())
    }

    fn look(&mut self, s: &mut Shared, cx: &mut Cx) {
        self.phase = Phase::Waiting;
        s.radio_on(cx);
        if let Some(bt) = &mut s.bt {
            bt.pair(true);
        }
    }
}

impl Default for Pair {
    fn default() -> Self {
        Pair::new()
    }
}

/// "123456" as "123 456", easier to compare.
fn spaced(code: u32) -> String {
    let s = format!("{code:06}");
    format!("{} {}", &s[..3], &s[3..])
}

impl Screen for Pair {
    fn title(&self) -> &str {
        words::PAIR
    }

    fn icon(&self) -> &str {
        "bluetooth"
    }

    fn bars(&self, s: &Shared) -> Bars {
        match Pair::asking(s) {
            Some(Ask::Enter) => Bars::top().hints([("Esc", "no"), ("Enter", "pair")]),
            _ => Bars::top(),
        }
    }

    fn typing(&self) -> bool {
        self.entering
    }

    fn start(&mut self, s: &mut Shared, cx: &mut Cx) {
        self.look(s, cx);
    }

    fn stop(&mut self, s: &mut Shared, _: &mut Cx) {
        if let Some(bt) = &mut s.bt {
            bt.pair(false);
        }
    }

    fn draw(&self, s: &Shared, ui: &mut Ui) {
        let host = self.host_name(s);
        let ask = Pair::asking(s);
        let code = match ask {
            Some(Ask::Confirm(c)) => Some((c, words::pair_confirm(&host))),
            Some(Ask::Show(c)) => Some((c, words::pair_show(&host))),
            _ => None,
        };
        if let Some((c, line)) = code {
            let keys: &[Hint] = if matches!(ask, Some(Ask::Confirm(_))) {
                &[
                    Hint {
                        key: "Enter",
                        label: "yes",
                    },
                    Hint {
                        key: "Esc",
                        label: "no",
                    },
                ]
            } else {
                &[Hint {
                    key: "Esc",
                    label: "cancel",
                }]
            };
            draw_code(ui, &spaced(c), &line, keys);
            return;
        }
        match ask {
            Some(Ask::Allow) => ui.notice(
                "bluetooth",
                None,
                &words::pair_allow(&host),
                "",
                &[("Enter", "pair"), ("Esc", "no")],
            ),
            Some(Ask::Enter) => {
                ui.note(&words::pair_enter(&host));
                ui.field(&self.code, "123456", true);
            }
            _ => {
                let name =
                    s.bt.as_ref()
                        .map(|b| b.name().to_string())
                        .unwrap_or_default();
                match self.phase {
                    Phase::Waiting => ui.notice(
                        "bluetooth",
                        None,
                        words::PAIR_WAIT,
                        &words::pair_how(&name),
                        &[],
                    ),
                    Phase::Ended => ui.notice(
                        "bluetooth",
                        Some(Tone::Warning),
                        words::PAIR_ENDED,
                        words::PAIR_AGAIN,
                        &[("Enter", "look again")],
                    ),
                    Phase::Cancelled => ui.notice(
                        "bluetooth",
                        Some(Tone::Warning),
                        words::PAIR_CANCELLED,
                        &words::pair_how(&name),
                        &[],
                    ),
                }
            }
        }
    }

    fn key(&mut self, s: &mut Shared, key: Key, cx: &mut Cx) -> Nav {
        match (Pair::asking(s), key) {
            (Some(Ask::Enter), key) => {
                if self.code.key(key) != Some(Edit::Submit) {
                    return Nav::Stay;
                }
                match self.code.text().trim().parse::<u32>() {
                    Ok(n) if n < 1_000_000 => {
                        if let Some(bt) = &mut s.bt {
                            bt.passkey(n);
                        }
                        self.entering = false;
                    }
                    _ => cx.toast("Six digits"),
                }
                self.code = TextField::new();
            }
            (Some(Ask::Confirm(_) | Ask::Allow), Key::Enter) => {
                if let Some(bt) = &mut s.bt {
                    bt.answer(true);
                }
            }
            (None, Key::Enter) if self.phase != Phase::Waiting => self.look(s, cx),
            _ => {}
        }
        Nav::Stay
    }

    fn back(&mut self, s: &mut Shared, _: &mut Cx) -> bool {
        // Esc says no to a question first; then it leaves.
        self.entering = false;
        match s.bt.as_mut() {
            Some(bt) if bt.asking().is_some() => {
                bt.answer(false);
                false
            }
            _ => true,
        }
    }

    fn event(&mut self, s: &mut Shared, e: &Event, _: &mut Cx) -> Nav {
        match e {
            Event::Pairing { device, ask } => {
                self.device = device.clone();
                self.phase = Phase::Waiting;
                self.entering = *ask == Ask::Enter;
            }
            Event::PairCancelled => {
                self.phase = Phase::Cancelled;
                self.entering = false;
            }
            Event::AdvertEnded => self.phase = Phase::Ended,
            Event::Paired(path) => {
                let Some(addr) = address_of(path) else {
                    return Nav::Stay;
                };
                let (name, system) = match s.device_at(path) {
                    Some(d) => (d.name.clone(), System::guess(&d.icon, &d.modalias)),
                    None => (addr.clone(), System::Windows),
                };
                if s.host(&addr).is_none() {
                    s.hosts.insert(0, Host::new(&addr, &name, system));
                    s.save_hosts();
                    s.sync_hosts();
                }
                return Nav::Replace(Box::new(Profile::new(s, &addr, true)));
            }
            _ => {}
        }
        Nav::Stay
    }
}

/// The pairing code, large, with what to do and the keys under it.
fn draw_code(ui: &mut Ui, code: &str, line: &str, keys: &[Hint]) {
    let a = ui.area();
    let t = *ui.theme();
    let c = ui.canvas();
    let line_h = t.m.line_h;
    let code_h = t.m.row_h + t.m.pad;
    let top = a.y + t.m.margin_v;
    widgets::text_centre(c, t.f.body, Rect::new(a.x, top, a.w, line_h), line, t.c.fg);
    let code_r = Rect::new(a.x, top + line_h + t.m.gap, a.w, code_h + t.m.gap);
    widgets::text_centre(c, t.f.big, code_r, code, t.c.accent);
    let kw = widgets::key_row_w(&t, keys);
    widgets::key_row(c, &t, a.x + (a.w - kw) / 2, code_r.bottom() + t.m.gap, keys);
}
