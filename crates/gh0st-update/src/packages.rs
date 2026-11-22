//! 6 List: every package the update brings, grouped by where it comes
//! from, and one package's details on Enter.

use crate::words;
use gh0st_app::prelude::*;
use gh0st_core::update::{self, Group, Package, Restart};

struct Line {
    package: Package,
    size: String,
    versions: String,
}

pub struct Packages {
    lines: Vec<Line>,
    list: List,
}

impl Packages {
    pub fn new(packages: &[Package]) -> Packages {
        let mut lines: Vec<Line> = packages
            .iter()
            .map(|p| Line {
                package: p.clone(),
                size: update::size_text(p.size),
                versions: words::versions(p),
            })
            .collect();
        lines.sort_by(|a, b| {
            (a.package.group, &a.package.name).cmp(&(b.package.group, &b.package.name))
        });
        Packages {
            lines,
            list: List::new(),
        }
    }
}

impl Page for Packages {
    fn title(&self) -> &str {
        "Packages"
    }

    fn icon(&self) -> &str {
        "list"
    }

    fn context(&self) -> Option<&str> {
        Some("Updates")
    }

    fn key(&mut self, key: Key, cx: &mut Cx) {
        if let Some(Action::Open(i)) = self.list.key(key) {
            if let Some(l) = self.lines.get(i) {
                cx.push(Details(l.package.clone()));
            }
        }
    }

    fn draw(&self, ui: &mut Ui) {
        if self.lines.is_empty() {
            ui.notice("circle-check", Some(Tone::Ok), "Nothing to update", "", &[]);
            return;
        }
        let rows: Vec<Row> = self
            .lines
            .iter()
            .map(|l| {
                Row::info(&l.package.name, &l.size)
                    .icon(words::group_icon(l.package.group))
                    .note(&l.versions)
            })
            .collect();
        ui.list(&self.list, &rows);
    }
}

/// One package: what it is, its versions, its size, and what installing it
/// needs.
pub struct Details(pub Package);

impl Page for Details {
    fn title(&self) -> &str {
        &self.0.name
    }

    fn icon(&self) -> &str {
        words::group_icon(self.0.group)
    }

    fn context(&self) -> Option<&str> {
        Some("Packages")
    }

    fn draw(&self, ui: &mut Ui) {
        let p = &self.0;
        if !p.summary.is_empty() {
            ui.text(&p.summary);
            ui.space(6);
        }
        ui.readout("Version", &words::versions(p));
        ui.readout("Download", &update::size_text(p.size));
        let from = match p.group {
            Group::Gh0st => "Gh0st OS",
            Group::Security => "Debian security",
            Group::System => "Debian",
        };
        ui.readout("From", from);
        let after = match p.restart {
            Restart::Device => "The device restarts",
            Restart::Shell => "Gh0st restarts",
            Restart::None => "Nothing",
        };
        ui.readout("After", after);
    }
}
