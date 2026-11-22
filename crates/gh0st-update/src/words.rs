//! What the app says: group names, short versions, times, and the line
//! under each group. Pure, so the tests read like the screen.

use gh0st_core::update::{Group, Package, State, is_kernel_set};

pub fn group_name(g: Group) -> &'static str {
    match g {
        Group::Gh0st => "Gh0st OS",
        Group::Security => "Security",
        Group::System => "System",
    }
}

pub fn group_icon(g: Group) -> &'static str {
    match g {
        Group::Gh0st => "gh0st-components",
        Group::Security => "shield",
        Group::System => "package",
    }
}

/// "0.5.0" from our "0.5.0+202611221733.35a14dd" (the build stamp goes),
/// "6.18.60" from a kernel set's "6.18.60-1+rpt1+m5.a3fd3e8+cpz1". Any
/// other version stays whole: a security fix often differs only in its
/// Debian revision.
pub fn short_version(v: &str) -> &str {
    if let Some((head, stamp)) = v.split_once('+') {
        if stamp.len() > 12 && stamp.as_bytes()[..12].iter().all(u8::is_ascii_digit) {
            return head;
        }
    }
    if v.contains("+rpt") {
        return v.split('-').next().unwrap_or(v);
    }
    v
}

/// The line under a group: what is in it, the important ones first.
pub fn group_note(packages: &[&Package]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for p in packages {
        if p.name == "gh0st" {
            parts.insert(
                0,
                format!(
                    "Gh0st {} → {}",
                    short_version(&p.from),
                    short_version(&p.to)
                ),
            );
        } else if p.name == "cpzeroos-kernel" {
            parts.push(format!("kernel {}", short_version(&p.to)));
        } else if !is_kernel_set(&p.name) {
            parts.push(p.name.clone());
        }
    }
    parts.join(", ")
}

/// What a package's versions read as: "0.4.1 → 0.5.0", "new 1.2".
pub fn versions(p: &Package) -> String {
    if p.from.is_empty() {
        format!("new {}", short_version(&p.to))
    } else {
        format!("{} → {}", short_version(&p.from), short_version(&p.to))
    }
}

/// "today at 09:12", "yesterday at 21:40", "on 3 Nov".
pub fn when(epoch: u64, now: u64, h24: bool) -> String {
    use gh0st_core::rt;
    let t = rt::local_at(epoch as i64);
    let day = |l: rt::Local| (l.year, l.yday);
    let today = day(rt::local_at(now as i64));
    let yesterday = day(rt::local_at(now as i64 - 86_400));
    let clock = rt::clock_format(t.hour, t.minute, h24);
    if day(t) == today {
        format!("today at {clock}")
    } else if day(t) == yesterday {
        format!("yesterday at {clock}")
    } else {
        format!("on {} {}", t.day, rt::month_short(t.month))
    }
}

/// "12 updates", "1 update".
pub fn updates(n: usize) -> String {
    if n == 1 {
        "1 update".into()
    } else {
        format!("{n} updates")
    }
}

/// The caption of the up-to-date screen: the version and the last check.
pub fn current_note(st: &State, gh0st: &str, now: u64, h24: bool) -> String {
    let mut s = format!(
        "Gh0st {} · {}",
        short_version(gh0st),
        channel_name(&st.channel)
    );
    if st.checked > 0 {
        s.push_str(&format!("\nChecked {}", when(st.checked, now, h24)));
    }
    s
}

pub fn channel_name(c: &str) -> &'static str {
    if c == "edge" { "Edge" } else { "Stable" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, from: &str, to: &str) -> Package {
        Package {
            name: name.into(),
            from: from.into(),
            to: to.into(),
            ..Package::default()
        }
    }

    #[test]
    fn versions_read_short() {
        assert_eq!(short_version("0.5.0+202611221733.35a14dd"), "0.5.0");
        assert_eq!(short_version("6.18.60-1+rpt1+m5.a3fd3e8+cpz1"), "6.18.60");
        assert_eq!(short_version("2.41-12+deb13u5"), "2.41-12+deb13u5");
        assert_eq!(short_version("1:9.2p1-2"), "1:9.2p1-2");
        assert_eq!(versions(&pkg("x", "", "1.2-3")), "new 1.2-3");
    }

    #[test]
    fn a_group_says_gh0st_first_and_the_kernel_once() {
        let g = pkg(
            "gh0st",
            "0.4.1+202611201200.1a2b3c4",
            "0.5.0+202611221733.35a14dd",
        );
        let k = pkg("cpzeroos-kernel", "", "6.18.60-1+rpt1+m5.a3+cpz2");
        let m = pkg("cpzeroos-panel-modules", "", "6.18.60-1");
        let t = pkg("gh0st-term", "", "0.5.0");
        assert_eq!(
            group_note(&[&t, &k, &m, &g]),
            "Gh0st 0.4.1 → 0.5.0, gh0st-term, kernel 6.18.60"
        );
    }

    #[test]
    fn counts_read_as_words() {
        assert_eq!(updates(1), "1 update");
        assert_eq!(updates(12), "12 updates");
    }
}
