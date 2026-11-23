//! Every Settings page, run as setup runs one: drawn in each palette under
//! the design lint, then left alone to count its wakes. The system tools
//! Settings asks are silent stubs first on PATH, so the host's own never
//! answer and every machine sees the same pages.

use gh0st_app::gfx::theme;
use gh0st_app::testing::Driver;
use gh0st_settings::app::{Embed, Settings};
use gh0st_settings::model::PageId;

mod common;
use common::stub_tools;

fn pages() -> Vec<PageId> {
    vec![
        PageId::Root,
        PageId::Wifi,
        PageId::Network,
        PageId::Bluetooth,
        PageId::Display,
        PageId::Sound,
        PageId::Appearance,
        PageId::Apps,
        PageId::System,
        PageId::TimeZone(None),
        PageId::Advanced,
        PageId::Swap,
        PageId::Country(None),
        PageId::Privacy,
        PageId::Updates,
        PageId::About,
        PageId::Look,
        PageId::CountryZones,
        PageId::Hotspot,
    ]
}

fn open(page: &PageId) -> Driver {
    stub_tools();
    let mut d = Driver::new(Settings::embedded(Embed {
        page: page.clone(),
        label: "Setup 1 of 1".into(),
        skip: false,
        done: false,
    }));
    // Past any toast from the start.
    d.wait(5_000);
    d
}

#[test]
fn every_page_keeps_the_design_rules_in_every_palette() {
    let mut found = Vec::new();
    for page in pages() {
        let mut d = open(&page);
        for p in theme::PALETTES {
            d.palette(p.name);
            found.extend(
                d.lint()
                    .into_iter()
                    .map(|f| format!("{page:?} {}: {f}", p.name)),
            );
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}

/// Idle, a page wakes for the minute's clock and for nothing else.
#[test]
fn an_idle_page_sleeps() {
    for page in pages() {
        let mut d = open(&page);
        let wakes = d.idle_wakes(600_000);
        assert!(wakes <= 10, "{page:?} woke {wakes} times in 10 min");
    }
}
