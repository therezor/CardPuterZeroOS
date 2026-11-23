//! What the Store says, in one place.

use gh0st_core::store::{Flags, Outcome};
use gh0st_core::update::size_text;

/// Help's About section.
pub const ABOUT: &str = "The apps and their list come from M5Stack. Gh0st \
signs a copy of the list so it arrives unchanged, but doesn't review the \
apps. An app's setup can run as root: the Store asks before installing one \
that does. No account, no device ID.";

/// The risks an app's package brings, short, then what it uses.
pub fn flags(f: Flags) -> (Vec<&'static str>, Vec<&'static str>) {
    let warn = [
        (Flags::SCRIPTS, "Setup runs as root"),
        (Flags::PATHS, "System files"),
        (Flags::SERVICE, "Service"),
    ]
    .into_iter()
    .filter(|(b, _)| f.has(*b))
    .map(|(_, w)| w)
    .collect();
    let uses = [
        (Flags::NETWORK, "network"),
        (Flags::CAMERA, "camera"),
        (Flags::MICROPHONE, "microphone"),
        (Flags::HARDWARE, "pins and Grove"),
        (Flags::DISPLAY, "external display"),
        (Flags::IMU, "motion sensor"),
    ]
    .into_iter()
    .filter(|(b, _)| f.has(*b))
    .map(|(_, w)| w)
    .collect();
    (warn, uses)
}

/// The risks as a sentence, for the install question: "It runs setup as
/// root and starts a service."
pub fn risks(f: Flags) -> String {
    let parts: Vec<&str> = [
        (Flags::SCRIPTS, "runs setup as root"),
        (Flags::PATHS, "changes system files"),
        (Flags::SERVICE, "starts a service"),
    ]
    .into_iter()
    .filter(|(b, _)| f.has(*b))
    .map(|(_, w)| w)
    .collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => format!("It {one}."),
        [rest @ .., last] => format!("It {} and {last}.", rest.join(", ")),
    }
}

/// "Uses network, camera".
pub fn uses(uses: &[&str]) -> String {
    if uses.is_empty() {
        String::new()
    } else {
        format!("Uses {}", uses.join(", "))
    }
}

/// A failed run, for a toast: what went wrong, in a few words.
pub fn outcome(o: Outcome, op: &str, title: &str) -> String {
    match o {
        Outcome::Offline => "No network: join Wi-Fi in Settings".into(),
        Outcome::BadSignature => "The app list failed its signature check".into(),
        Outcome::Stale => "The app list is out of date. Try again later".into(),
        Outcome::Clock => "The clock is behind: set it in Settings".into(),
        Outcome::Checksum => format!("{title} didn't match the list. Nothing changed"),
        Outcome::Depends => format!("{title} needs a package Gh0st OS lacks"),
        Outcome::Locked => "Another install is running. Try again soon".into(),
        Outcome::Space => "Not enough space".into(),
        Outcome::Battery => "Battery low: charge to install".into(),
        _ if op == "refresh" => "Couldn't get the app list".into(),
        _ if op == "remove" => format!("Couldn't remove {title}"),
        _ => format!("Couldn't install {title}"),
    }
}

/// "AcidZero · 0.2.0 · 58 KB".
pub fn line(author: &str, version: &str, size: u64) -> String {
    let mut parts = Vec::new();
    if !author.is_empty() {
        parts.push(author.to_string());
    }
    parts.push(version.to_string());
    parts.push(size_text(size));
    parts.join(" \u{b7} ")
}

pub fn apps(n: usize) -> String {
    match n {
        1 => "1 app".into(),
        n => format!("{n} apps"),
    }
}

pub fn updates(n: usize) -> String {
    match n {
        1 => "1 app update".into(),
        n => format!("{n} app updates"),
    }
}

/// How long ago the list was fetched: "today", "3 days ago".
pub fn age(secs: u64) -> String {
    match secs / 86_400 {
        0 => "today".into(),
        1 => "yesterday".into(),
        n => format!("{n} days ago"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_split_risks_from_uses() {
        let (warn, uses_) = flags(Flags::parse("scripts,network,camera"));
        assert_eq!(warn, ["Setup runs as root"]);
        assert_eq!(uses(&uses_), "Uses network, camera");
        assert_eq!(uses(&[]), "");
    }

    #[test]
    fn risks_read_as_a_sentence() {
        assert_eq!(risks(Flags::parse("network")), "");
        assert_eq!(risks(Flags::parse("service")), "It starts a service.");
        assert_eq!(
            risks(Flags::parse("scripts,paths,service")),
            "It runs setup as root, changes system files and starts a service."
        );
    }

    #[test]
    fn a_line_skips_a_missing_author() {
        assert_eq!(line("", "1.0", 2048), "1.0 \u{b7} 2 kB");
    }
}
