//! What the app keeps, one record per line in its data folder: the hosts
//! it paired and the user's snippets. Tabs, newlines and backslashes are
//! escaped so a field stays one field. No secret goes here.

use std::path::Path;

use crate::host::System;

/// A computer or phone paired with this keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    /// Its Bluetooth address, `AA:BB:CC:DD:EE:FF`.
    pub address: String,
    /// The user's name for it, or the name it gave.
    pub name: String,
    pub system: System,
    /// Stays connected while the app runs behind another. Off for a phone,
    /// which hides its own keyboard while one is connected.
    pub stay: bool,
    /// Seconds since 1970 it was last used, for the order and the next
    /// start's guess.
    pub used: u64,
}

impl Host {
    pub fn new(address: &str, name: &str, system: System) -> Host {
        Host {
            address: address.into(),
            name: name.into(),
            system,
            stay: !system.handheld(),
            used: 0,
        }
    }

    /// BlueZ's object path for this host on the first adapter.
    pub fn path(&self) -> String {
        path_of(&self.address)
    }

    fn line(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}",
            esc(&self.address),
            esc(&self.name),
            self.system.id(),
            if self.stay { "stay" } else { "drop" },
            self.used
        )
    }

    fn parse(line: &str) -> Option<Host> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 5 || f[0].is_empty() {
            return None;
        }
        Some(Host {
            address: unesc(f[0]),
            name: unesc(f[1]),
            system: System::from_id(f[2]),
            stay: f[3] == "stay",
            used: f[4].parse().unwrap_or(0),
        })
    }
}

/// A text the user types on a host with one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub name: String,
    pub text: String,
}

impl Snippet {
    fn line(&self) -> String {
        format!("{}\t{}", esc(&self.name), esc(&self.text))
    }

    fn parse(line: &str) -> Option<Snippet> {
        let (name, text) = line.split_once('\t')?;
        Some(Snippet {
            name: unesc(name),
            text: unesc(text),
        })
    }
}

/// BlueZ's object path for an address on the first adapter.
pub fn path_of(address: &str) -> String {
    format!("/org/bluez/hci0/dev_{}", address.replace(':', "_"))
}

pub const HOSTS: &str = "hosts";
pub const SNIPPETS: &str = "snippets";

pub fn hosts(dir: &Path) -> Vec<Host> {
    read(&dir.join(HOSTS))
        .iter()
        .filter_map(|l| Host::parse(l))
        .collect()
}

pub fn save_hosts(dir: &Path, hosts: &[Host]) -> std::io::Result<()> {
    write(&dir.join(HOSTS), hosts.iter().map(Host::line))
}

pub fn snippets(dir: &Path) -> Vec<Snippet> {
    read(&dir.join(SNIPPETS))
        .iter()
        .filter_map(|l| Snippet::parse(l))
        .collect()
}

pub fn save_snippets(dir: &Path, snippets: &[Snippet]) -> std::io::Result<()> {
    write(&dir.join(SNIPPETS), snippets.iter().map(Snippet::line))
}

fn read(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

/// The whole list: a temp file, then a rename, so a crash keeps the old.
fn write(path: &Path, lines: impl Iterator<Item = String>) -> std::io::Result<()> {
    let mut text = String::new();
    for l in lines {
        text.push_str(&l);
        text.push('\n');
    }
    gh0st_core::config::write_atomic(path, text.as_bytes())
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_snippets_round_trip() {
        let dir = std::env::temp_dir().join(format!("gh0st-kb-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut mac = Host::new("AA:BB:CC:DD:EE:01", "Work\tMac", System::Mac);
        mac.used = 42;
        let phone = Host::new("AA:BB:CC:DD:EE:02", "Pixel", System::Android);
        assert!(!phone.stay, "a phone lets go behind other apps");
        save_hosts(&dir, &[mac.clone(), phone.clone()]).unwrap();
        assert_eq!(hosts(&dir), [mac.clone(), phone]);
        assert_eq!(mac.path(), "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_01");
        let s = Snippet {
            name: "Address".into(),
            text: "1 Main St\nSpringfield\\".into(),
        };
        save_snippets(&dir, std::slice::from_ref(&s)).unwrap();
        assert_eq!(snippets(&dir), [s]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
