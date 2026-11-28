//! Unused packages: what `apt-get autoremove` would take. The check lists
//! them; the install the user starts removes them after the upgrade, and
//! only the ones the check showed (docs/updates.md, Unused packages).

/// Never unused, nor anything they need: Gh0st OS itself, and the kernel
/// set, since removing a kernel rewrites `/boot/firmware`.
const KEEP: &[&str] = &["^gh0st", "^linux-image-", "^raspi-firmware$"];

/// apt options that keep [`KEEP`] and `also`, by exact name, out of
/// autoremove. apt keeps what a kept package needs too.
pub fn keep_args(also: &[String]) -> Vec<String> {
    KEEP.iter()
        .map(|p| p.to_string())
        .chain(also.iter().map(|n| format!("^{}$", escape(n))))
        .flat_map(|p| ["-o".to_string(), format!("APT::NeverAutoRemove::={p}")])
        .collect()
}

/// A package name as a regex that matches only it: `.` and `+` are its
/// only characters with a meaning there.
fn escape(name: &str) -> String {
    name.chars()
        .flat_map(|c| match c {
            '.' | '+' => vec!['\\', c],
            c => vec![c],
        })
        .collect()
}

/// `Remv name [version]` lines of a simulation: (name, version).
pub fn removes(sim: &str) -> Vec<(String, String)> {
    sim.lines()
        .filter_map(|l| l.strip_prefix("Remv "))
        .filter_map(|r| {
            let mut w = r.split_whitespace();
            let name = w.next()?.to_string();
            let version = w
                .next()
                .map(|v| v.trim_start_matches('[').trim_end_matches(']'))
                .unwrap_or("")
                .to_string();
            Some((name, version))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_simulation_names_what_it_removes() {
        let sim = "Reading package lists...\nRemv libavcodec61 [7:7.1.1-1]\nRemv gpgconf [2.4.7]\n";
        assert_eq!(
            removes(sim),
            [
                ("libavcodec61".to_string(), "7:7.1.1-1".to_string()),
                ("gpgconf".to_string(), "2.4.7".to_string())
            ]
        );
        assert!(removes("0 upgraded, 0 newly installed, 0 to remove\n").is_empty());
    }

    #[test]
    fn kept_names_match_only_themselves() {
        let args = keep_args(&["libstdc++6".into(), "python3.13".into()]);
        assert_eq!(args.len(), 2 * (KEEP.len() + 2));
        assert!(
            args.iter()
                .all(|a| a == "-o" || a.starts_with("APT::NeverAutoRemove::="))
        );
        assert!(args.contains(&"APT::NeverAutoRemove::=^libstdc\\+\\+6$".to_string()));
        assert!(args.contains(&"APT::NeverAutoRemove::=^python3\\.13$".to_string()));
    }
}
