//! Reading apt: the simulated upgrade, package records, Status-Fd lines and
//! the errors worth words. Pure functions; the fixtures in `testdata` are
//! real output from the release image (`docs/updates.md`, Tested).

use gh0st_core::update::Outcome;

/// One `Inst` line of `apt-get -s upgrade`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inst {
    pub name: String,
    /// Empty for a package new to the device.
    pub from: String,
    pub to: String,
    /// The archive's origin: `Gh0st`, `Debian`, `Debian-Security`.
    pub origin: String,
}

/// The upgrades `apt-get -s` would make, and how many it holds back.
pub fn simulate(text: &str) -> (Vec<Inst>, u32) {
    let mut out = Vec::new();
    let mut held = 0;
    let mut in_held = false;
    for line in text.lines() {
        if line.starts_with("The following packages have been kept back") {
            in_held = true;
            continue;
        }
        if in_held {
            if line.starts_with(' ') {
                held += line.split_whitespace().count() as u32;
                continue;
            }
            in_held = false;
        }
        if let Some(i) = inst(line) {
            out.push(i);
        }
    }
    (out, held)
}

/// `Inst libc6 [2.41-12+deb13u4] (2.41-12+deb13u5 Debian:13.8/stable [arm64])`
fn inst(line: &str) -> Option<Inst> {
    let rest = line.strip_prefix("Inst ")?;
    let (name, rest) = rest.split_once(' ')?;
    let (from, rest) = match rest.strip_prefix('[') {
        Some(r) => {
            let (v, r) = r.split_once("] ")?;
            (v.to_string(), r)
        }
        None => (String::new(), rest),
    };
    let inner = rest.strip_prefix('(')?;
    let inner = &inner[..inner.find(')')?];
    let mut words = inner.split_whitespace();
    let to = words.next()?.to_string();
    let origin = words
        .next()
        .and_then(|o| o.split(':').next())
        .unwrap_or("")
        .to_string();
    Some(Inst {
        name: name.into(),
        from,
        to,
        origin,
    })
}

/// What `apt-cache show` says of one version.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    pub version: String,
    /// Bytes to download.
    pub size: u64,
    /// Kilobytes once installed.
    pub installed_kb: u64,
    /// The description's first line.
    pub summary: String,
}

pub fn records(text: &str) -> Vec<Record> {
    let mut out = Vec::new();
    let mut r = Record::default();
    for line in text.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if !r.name.is_empty() {
                out.push(std::mem::take(&mut r));
            }
            continue;
        }
        let Some((k, v)) = line.split_once(": ") else {
            continue;
        };
        match k {
            "Package" => r.name = v.into(),
            "Version" => r.version = v.into(),
            "Size" => r.size = v.trim().parse().unwrap_or(0),
            "Installed-Size" => r.installed_kb = v.trim().parse().unwrap_or(0),
            "Description" | "Description-en" if r.summary.is_empty() => r.summary = v.into(),
            _ => {}
        }
    }
    out
}

/// One line apt writes to its Status-Fd.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Downloading: file n of m, and apt's percent (0 while it fetches
    /// indexes, so the caller uses n of m then).
    Dl {
        percent: f32,
        n: u32,
        m: u32,
    },
    /// dpkg at work on `pkg`: "Unpacking libc6 (arm64)".
    Pm {
        pkg: String,
        percent: f32,
        what: String,
    },
    PmError {
        pkg: String,
        why: String,
    },
    Other,
}

pub fn status(line: &str) -> Status {
    let mut f = line.splitn(4, ':');
    let kind = f.next().unwrap_or("");
    let a = f.next().unwrap_or("");
    let percent = f.next().and_then(|p| p.parse().ok()).unwrap_or(0.0);
    let msg = f.next().unwrap_or("").trim();
    match kind {
        "dlstatus" => {
            // "Retrieving file 2 of 14"
            let mut nums = msg.split_whitespace().filter_map(|w| w.parse::<u32>().ok());
            let n = nums.next().unwrap_or(0);
            let m = nums.next().unwrap_or(0);
            Status::Dl { percent, n, m }
        }
        "pmstatus" => Status::Pm {
            pkg: a.into(),
            percent,
            what: msg.into(),
        },
        "pmerror" => Status::PmError {
            pkg: a.into(),
            why: msg.into(),
        },
        _ => Status::Other,
    }
}

/// Turns a failed apt run's output into an outcome and words for the user.
pub fn failure(out: &str) -> (Outcome, String) {
    let has = |s: &str| out.contains(s);
    if has("Temporary failure resolving")
        || has("Could not resolve")
        || has("Network is unreachable")
    {
        return (Outcome::Offline, "No network".into());
    }
    if has("is not valid yet") {
        return (Outcome::Clock, "The clock is behind".into());
    }
    if has("Could not get lock") || has("Unable to acquire the dpkg frontend lock") {
        return (Outcome::Locked, "Another install is running".into());
    }
    if has("No space left on device") || has("You don't have enough free space") {
        return (Outcome::Space, "Not enough free space".into());
    }
    if has("is expired") {
        return (
            Outcome::Error,
            "The update server's signature is out of date".into(),
        );
    }
    if has("NO_PUBKEY")
        || has("is not signed")
        || has("couldn't be verified")
        || has("verification failed")
    {
        return (
            Outcome::Error,
            "The update source isn't signed right".into(),
        );
    }
    if has("Hash Sum mismatch") || has("404") || has("File not found") {
        return (
            Outcome::Error,
            "The server changed mid-update. Try again".into(),
        );
    }
    let last = out
        .lines()
        .rev()
        .find(|l| l.starts_with("E: "))
        .map(|l| l.trim_start_matches("E: ").to_string());
    (Outcome::Error, last.unwrap_or_else(|| "apt failed".into()))
}

/// The sources an `apt-get update` names as failed, by URL: its `Err:`
/// lines and the `E:` and `W:` lines that carry one.
pub fn failed_sources(out: &str) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for line in out.lines() {
        let l = line.trim_start();
        let url = if let Some(rest) = l.strip_prefix("Err:") {
            rest.split_whitespace().nth(1)
        } else if l.starts_with("E: ") || l.starts_with("W: ") {
            l.split(['\'', ' '])
                .find(|w| w.starts_with("https://") || w.starts_with("http://"))
        } else {
            None
        };
        if let Some(u) = url {
            let u = u.trim_end_matches(':').to_string();
            if !urls.contains(&u) {
                urls.push(u);
            }
        }
    }
    urls
}

/// The hosts of `failed` when none of it is a source the device updates
/// from (`trusted`, its `URIs:`), or None. Only then may a check go on
/// without them: a broken source someone added must not stop updates,
/// and one of ours or Debian's failing still fails the check.
pub fn only_foreign(failed: &[String], trusted: &[String]) -> Option<Vec<String>> {
    if failed.is_empty() {
        return None;
    }
    let ours = |u: &str| {
        trusted
            .iter()
            .any(|t| u.starts_with(t.trim_end_matches('/')))
    };
    if failed.iter().any(|u| ours(u)) {
        return None;
    }
    let mut hosts: Vec<String> = failed
        .iter()
        .map(|u| {
            u.split("://")
                .nth(1)
                .unwrap_or(u)
                .split('/')
                .next()
                .unwrap_or(u)
                .to_string()
        })
        .collect();
    hosts.sort();
    hosts.dedup();
    Some(hosts)
}

/// The `URIs:` of deb822 source files.
pub fn uris(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case("uris").then_some(v)
        })
        .flat_map(|v| v.split_whitespace().map(String::from))
        .collect()
}

/// Worth one fresh `apt-get update` and a second try.
pub fn retryable(out: &str) -> bool {
    [
        "Hash Sum mismatch",
        "404  Not Found",
        "404 Not Found",
        "File not found",
    ]
    .iter()
    .any(|s| out.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What apt said on a device where an app added Kismet's source with
    /// an empty key (issue #7).
    const UNSIGNED: &str = "Hit:1 https://deb.debian.org/debian trixie InRelease\n\
        Get:4 https://www.kismetwireless.net/repos/apt/release/trixie trixie InRelease [7500 B]\n\
        Err:4 https://www.kismetwireless.net/repos/apt/release/trixie trixie InRelease\n  \
        Sub-process /usr/bin/sqv returned an error code (1), error message is: Missing key 5345B8BF43403B93, which is needed to verify signature.\n\
        W: OpenPGP signature verification failed: https://www.kismetwireless.net/repos/apt/release/trixie trixie InRelease: Sub-process /usr/bin/sqv returned an error code (1)\n\
        E: The repository 'https://www.kismetwireless.net/repos/apt/release/trixie trixie InRelease' is not signed.\n";

    #[test]
    fn a_broken_source_someone_added_is_skipped_and_ours_never_is() {
        let trusted = uris(
            "Types: deb\nURIs: https://deb.debian.org/debian\n\n\
             URIs: https://therezor.github.io/CardPuterZeroOS/apt\n",
        );
        let failed = failed_sources(UNSIGNED);
        assert_eq!(
            failed,
            ["https://www.kismetwireless.net/repos/apt/release/trixie"]
        );
        assert_eq!(
            only_foreign(&failed, &trusted),
            Some(vec!["www.kismetwireless.net".to_string()])
        );
        let offline = "Err:1 https://deb.debian.org/debian trixie InRelease\n  Temporary failure resolving 'deb.debian.org'\n\
            Err:2 https://www.kismetwireless.net/repos/apt/release/trixie trixie InRelease\n";
        assert_eq!(only_foreign(&failed_sources(offline), &trusted), None);
        let ours = "E: Failed to fetch https://therezor.github.io/CardPuterZeroOS/apt/dists/stable/InRelease  404\n";
        assert_eq!(only_foreign(&failed_sources(ours), &trusted), None);
        assert_eq!(
            only_foreign(&[], &trusted),
            None,
            "no source named: a real failure"
        );
    }

    const SIMULATE: &str = include_str!("testdata/simulate.txt");
    const SHOW: &str = include_str!("testdata/show.txt");
    const UPDATE: &str = include_str!("testdata/update.status");
    const INSTALL: &str = include_str!("testdata/install.status");

    #[test]
    fn the_simulated_upgrade_lists_each_package_with_its_origin() {
        let (list, held) = simulate(SIMULATE);
        assert_eq!(list.len(), 14);
        assert_eq!(held, 0);
        assert_eq!(
            list[0],
            Inst {
                name: "libc6".into(),
                from: "2.41-12+deb13u4".into(),
                to: "2.41-12+deb13u5".into(),
                origin: "Debian".into(),
            }
        );
        let gh0st = list.iter().find(|i| i.name == "gh0st").unwrap();
        assert_eq!(gh0st.origin, "Gh0st");
        assert_eq!(gh0st.to, "0.1.0+202611221733.35a14dd.dirty");
    }

    #[test]
    fn held_back_and_new_packages_read_too() {
        let text = "The following packages have been kept back:\n  gh0st-kernel gh0st-panel-modules\n\
                    Inst newpkg (1.0 Debian-Security:13/stable-security [arm64])\n\
                    Inst gh0st-kernel [1] (2 Gh0st:edge [arm64]) []\n";
        let (list, held) = simulate(text);
        assert_eq!(held, 2);
        assert_eq!(list[0].from, "");
        assert_eq!(list[0].origin, "Debian-Security");
        assert_eq!(list[1].to, "2");
    }

    #[test]
    fn records_give_size_and_summary() {
        let r = records(SHOW);
        let libc = r.iter().find(|r| r.name == "libc6").unwrap();
        assert_eq!(libc.version, "2.41-12+deb13u5");
        assert_eq!(libc.size, 2_488_872);
        assert_eq!(libc.installed_kb, 23_770);
        assert_eq!(libc.summary, "GNU C Library: Shared libraries");
        assert_eq!(r.len(), 14);
    }

    #[test]
    fn status_lines_from_a_real_update_and_install() {
        let dl: Vec<Status> = UPDATE.lines().map(status).collect();
        assert!(dl.iter().all(|s| matches!(s, Status::Dl { .. })));
        assert_eq!(
            dl.last(),
            Some(&Status::Dl {
                percent: 100.0,
                n: 12,
                m: 12
            })
        );
        let pm: Vec<Status> = INSTALL.lines().map(status).collect();
        assert!(pm.contains(&Status::Pm {
            pkg: "libc6".into(),
            percent: 1.7544,
            what: "Unpacking libc6 (arm64)".into(),
        }));
        assert!(
            pm.iter()
                .any(|s| matches!(s, Status::Dl { percent, .. } if *percent > 0.0))
        );
    }

    #[test]
    fn failures_have_words() {
        let f = |s| failure(s).0;
        assert_eq!(
            f(
                "Err:1 https://deb.debian.org trixie InRelease\n  Temporary failure resolving 'deb.debian.org'"
            ),
            Outcome::Offline
        );
        assert_eq!(
            f("E: Release file for https://x/InRelease is not valid yet (invalid for another 2d)"),
            Outcome::Clock
        );
        assert_eq!(
            f(
                "E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 812 (apt-get)"
            ),
            Outcome::Locked
        );
        assert_eq!(
            failure("E: Release file for https://x/InRelease is expired (invalid since 2d)").1,
            "The update server's signature is out of date"
        );
        assert_eq!(failure("E: Something else broke").1, "Something else broke");
        assert!(retryable("E: Failed to fetch x  Hash Sum mismatch"));
    }
}
