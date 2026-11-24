//! Built-in shortcuts for each host system: the combinations people use
//! most, and the ones that are awkward on this keyboard (Win+L, Cmd+Space).

use crate::hid::usage::{consumer, key, mods};
use crate::host::System;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Send {
    /// Modifiers and a key; a key of 0 taps the modifiers alone.
    Keys(u8, u8),
    /// A consumer key, as Android's Back and Home.
    Media(u16),
}

#[derive(Clone, Copy, Debug)]
pub struct Shortcut {
    pub label: &'static str,
    pub send: Send,
}

const fn k(label: &'static str, m: u8, usage: u8) -> Shortcut {
    Shortcut {
        label,
        send: Send::Keys(m, usage),
    }
}

const fn media(label: &'static str, usage: u16) -> Shortcut {
    Shortcut {
        label,
        send: Send::Media(usage),
    }
}

const CTRL: u8 = mods::LCTRL;
const SHIFT: u8 = mods::LSHIFT;
const ALT: u8 = mods::LALT;
const GUI: u8 = mods::LGUI;

const WINDOWS: &[Shortcut] = &[
    k("Copy", CTRL, key::C),
    k("Paste", CTRL, key::V),
    k("Cut", CTRL, key::X),
    k("Undo", CTRL, key::Z),
    k("Redo", CTRL, key::Y),
    k("Select all", CTRL, key::A),
    k("Switch app", ALT, key::TAB),
    k("Start menu", GUI, 0),
    k("Lock screen", GUI, key::L),
    k("Screenshot", GUI | SHIFT, key::S),
    k("Show desktop", GUI, key::D),
    k("Task Manager", CTRL | SHIFT, key::ESC),
    k("Close window", ALT, key::F4),
    k("Find", CTRL, key::F),
    k("Save", CTRL, key::S),
];

const MAC: &[Shortcut] = &[
    k("Copy", GUI, key::C),
    k("Paste", GUI, key::V),
    k("Cut", GUI, key::X),
    k("Undo", GUI, key::Z),
    k("Redo", GUI | SHIFT, key::Z),
    k("Select all", GUI, key::A),
    k("Switch app", GUI, key::TAB),
    k("Spotlight", GUI, key::SPACE),
    k("Lock screen", CTRL | GUI, key::Q),
    k("Screenshot", GUI | SHIFT, key::N4),
    k("Force quit", GUI | mods::LALT, key::ESC),
    k("Close window", GUI, key::W),
    k("Quit app", GUI, key::Q),
    k("Find", GUI, key::F),
    k("Save", GUI, key::S),
];

const LINUX: &[Shortcut] = &[
    k("Copy", CTRL, key::C),
    k("Paste", CTRL, key::V),
    k("Cut", CTRL, key::X),
    k("Undo", CTRL, key::Z),
    k("Redo", CTRL | SHIFT, key::Z),
    k("Select all", CTRL, key::A),
    k("Switch app", ALT, key::TAB),
    k("Activities", GUI, 0),
    k("Lock screen", GUI, key::L),
    k("Screenshot", 0, key::PRINT),
    k("Terminal", CTRL | ALT, key::T),
    k("Copy in a terminal", CTRL | SHIFT, key::C),
    k("Paste in a terminal", CTRL | SHIFT, key::V),
    k("Close window", ALT, key::F4),
    k("Save", CTRL, key::S),
];

const CHROMEOS: &[Shortcut] = &[
    k("Copy", CTRL, key::C),
    k("Paste", CTRL, key::V),
    k("Cut", CTRL, key::X),
    k("Undo", CTRL, key::Z),
    k("Redo", CTRL | SHIFT, key::Z),
    k("Select all", CTRL, key::A),
    k("Switch app", ALT, key::TAB),
    k("Launcher", GUI, 0),
    k("Lock screen", GUI, key::L),
    k("New tab", CTRL, key::T),
    k("Close tab", CTRL, key::W),
];

const ANDROID: &[Shortcut] = &[
    media("Back", consumer::AC_BACK),
    media("Home", consumer::AC_HOME),
    k("Recent apps", ALT, key::TAB),
    k("Notifications", GUI, key::N),
    k("Copy", CTRL, key::C),
    k("Paste", CTRL, key::V),
    k("Cut", CTRL, key::X),
    k("Undo", CTRL, key::Z),
    k("Select all", CTRL, key::A),
];

const IOS: &[Shortcut] = &[
    k("Home", GUI, key::H),
    k("Spotlight", GUI, key::SPACE),
    k("Switch app", GUI, key::TAB),
    k("Copy", GUI, key::C),
    k("Paste", GUI, key::V),
    k("Cut", GUI, key::X),
    k("Undo", GUI, key::Z),
    k("Select all", GUI, key::A),
    k("Screenshot", GUI | SHIFT, key::N3),
];

pub fn of(system: System) -> &'static [Shortcut] {
    match system {
        System::Windows => WINDOWS,
        System::Mac => MAC,
        System::Linux => LINUX,
        System::ChromeOs => CHROMEOS,
        System::Android => ANDROID,
        System::Ios => IOS,
    }
}

/// The modifiers as the host's keycaps name them, in the usual order.
pub fn mod_words(system: System, m: u8) -> Vec<String> {
    let gui = match system {
        System::Mac | System::Ios => "Cmd",
        System::Windows => "Win",
        System::ChromeOs => "Search",
        System::Linux | System::Android => "Super",
    };
    let mut out = Vec::new();
    if m & (mods::LCTRL | mods::RCTRL) != 0 {
        out.push("Ctrl".to_string());
    }
    if m & mods::LALT != 0 {
        out.push(if system.apple() { "Opt" } else { "Alt" }.to_string());
    }
    if m & mods::RALT != 0 {
        out.push(if system.apple() { "Opt" } else { "AltGr" }.to_string());
    }
    if m & mods::SHIFT != 0 {
        out.push("Shift".to_string());
    }
    if m & (mods::LGUI | mods::RGUI) != 0 {
        out.push(gui.to_string());
    }
    out
}

/// What the shortcut presses, as the host's keycaps say: "Ctrl Shift Esc".
pub fn keys(system: System, send: Send) -> String {
    let (m, usage) = match send {
        Send::Keys(m, u) => (m, u),
        Send::Media(_) => return String::new(),
    };
    let mut out = mod_words(system, m);
    let name = key_name(usage);
    if !name.is_empty() {
        out.push(name);
    }
    out.join(" ")
}

fn key_name(usage: u8) -> String {
    match usage {
        0 => String::new(),
        key::A..=key::Z => ((b'A' + usage - key::A) as char).to_string(),
        0x1e..=0x26 => ((b'1' + usage - 0x1e) as char).to_string(),
        0x27 => "0".into(),
        key::ESC => "Esc".into(),
        key::TAB => "Tab".into(),
        key::SPACE => "Space".into(),
        key::PRINT => "PrtSc".into(),
        key::F4 => "F4".into(),
        key::F5 => "F5".into(),
        key::ENTER => "Enter".into(),
        _ => "?".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_system_has_copy_and_paste() {
        for s in System::ALL {
            let labels: Vec<&str> = of(s).iter().map(|x| x.label).collect();
            assert!(
                labels.contains(&"Copy") && labels.contains(&"Paste"),
                "{s:?}"
            );
        }
    }

    #[test]
    fn keys_read_as_the_keycaps_say() {
        assert_eq!(
            keys(System::Windows, Send::Keys(CTRL | SHIFT, key::ESC)),
            "Ctrl Shift Esc"
        );
        assert_eq!(
            keys(System::Mac, Send::Keys(GUI | SHIFT, key::N4)),
            "Shift Cmd 4"
        );
        assert_eq!(keys(System::Windows, Send::Keys(GUI, 0)), "Win");
        assert_eq!(keys(System::Android, Send::Media(consumer::AC_BACK)), "");
    }
}
