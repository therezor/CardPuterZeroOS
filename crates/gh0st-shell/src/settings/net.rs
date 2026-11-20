//! nmcli's terse output read into Settings state. Only the parsing lives
//! here; the shell runs nmcli as jobs. Ported from pt35-menu.

use super::pages::{Iface, Network};

/// One line of terse output split into fields. A `:` inside a field comes
/// escaped as `\:`, and a backslash as `\\`.
pub fn fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        let c = match c {
            '\\' => match chars.next() {
                Some(next) => next,
                None => break,
            },
            ':' => {
                out.push(String::new());
                continue;
            }
            c => c,
        };
        if let Some(f) = out.last_mut() {
            f.push(c);
        }
    }
    out
}

/// What joining a network needs, from the SECURITY column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    Open,
    /// WPA, WPA3 or WEP: one password.
    Password,
    /// 802.1X wants a user name too.
    Enterprise,
}

impl Security {
    pub fn from_nmcli(field: &str) -> Security {
        if field.contains("802.1X") {
            Security::Enterprise
        } else if field.contains("WPA") || field.contains("SAE") || field.contains("WEP") {
            Security::Password
        } else {
            // Empty, `--`, or OWE, which encrypts without a password.
            Security::Open
        }
    }
}

/// Signal 0-100 as 0-4 bars.
pub fn bars(signal: i32) -> u8 {
    match signal {
        75.. => 4,
        50..=74 => 3,
        25..=49 => 2,
        1..=24 => 1,
        _ => 0,
    }
}

/// `nmcli -t -f IN-USE,SIGNAL,SECURITY,SSID device wifi list`: one row per
/// name, strongest kept, the one in use first, then by strength.
pub fn parse_list(out: &str, saved: &[String]) -> Vec<Network> {
    let mut seen: Vec<(Network, i32)> = Vec::new();
    for line in out.lines() {
        let f = fields(line);
        let field = |i: usize| f.get(i).map(String::as_str).unwrap_or("");
        let active = field(0) == "*";
        let signal: i32 = field(1).parse().unwrap_or(0);
        let security = Security::from_nmcli(field(2));
        // Not trimmed: a leading or trailing space is part of the name.
        let ssid = field(3).to_string();
        if ssid.is_empty() {
            continue;
        }
        match seen.iter_mut().find(|(n, _)| n.ssid == ssid) {
            Some((n, best)) => {
                n.active |= active;
                if signal > *best {
                    *best = signal;
                    n.bars = bars(signal);
                }
            }
            None => seen.push((
                Network {
                    known: saved.contains(&ssid),
                    ssid,
                    bars: bars(signal),
                    secure: security != Security::Open,
                    active,
                    enterprise: security == Security::Enterprise,
                },
                signal,
            )),
        }
    }
    seen.sort_by_key(|(n, s)| (!n.active, std::cmp::Reverse(*s)));
    seen.into_iter().map(|(n, _)| n).collect()
}

/// `nmcli -t -f NAME,TYPE connection show`: the Wi-Fi profiles' names, which
/// NetworkManager sets to the network's name.
pub fn parse_profiles(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| {
            let f = fields(l);
            (f.get(1).map(String::as_str) == Some("802-11-wireless")).then(|| f[0].clone())
        })
        .collect()
}

/// `nmcli -t -f WIFI radio`.
pub fn parse_radio(out: &str) -> Option<bool> {
    match out.trim() {
        "enabled" => Some(true),
        "disabled" => Some(false),
        _ => None,
    }
}

/// `nmcli -t -f GENERAL.DEVICE,GENERAL.TYPE,GENERAL.STATE,GENERAL.CONNECTION,
/// GENERAL.HWADDR,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS device show`: blocks of
/// `KEY:value` lines, one block per device. Loopback and Wi-Fi P2P go.
pub fn parse_devices(out: &str) -> Vec<Iface> {
    let mut out_list = Vec::new();
    let mut cur: Option<(Iface, String)> = None;
    let mut dns: Vec<String> = Vec::new();
    let mut finish = |cur: &mut Option<(Iface, String)>, dns: &mut Vec<String>| {
        if let Some((mut i, kind)) = cur.take() {
            i.dns = dns.join(", ");
            if kind != "loopback" && kind != "wifi-p2p" && !i.name.is_empty() {
                out_list.push(i);
            }
        }
        dns.clear();
    };
    for line in out.lines() {
        let f = fields(line);
        let (Some(key), Some(value)) = (f.first(), f.get(1)) else {
            continue;
        };
        let key = key.split('[').next().unwrap_or(key);
        if key == "GENERAL.DEVICE" {
            finish(&mut cur, &mut dns);
            cur = Some((
                Iface {
                    name: value.clone(),
                    ..Iface::default()
                },
                String::new(),
            ));
            continue;
        }
        let Some((i, kind)) = cur.as_mut() else {
            continue;
        };
        let value = value.clone();
        match key {
            "GENERAL.TYPE" => *kind = value,
            // "100 (connected)": keep the words.
            "GENERAL.STATE" => {
                i.state = value
                    .split_once('(')
                    .map(|(_, s)| s.trim_end_matches(')').to_string())
                    .unwrap_or(value);
            }
            "GENERAL.CONNECTION" if value != "--" => i.connection = value,
            "GENERAL.HWADDR" => i.mac = value.to_lowercase(),
            "IP4.ADDRESS" if i.ipv4.is_empty() => i.ipv4 = value,
            "IP4.GATEWAY" if value != "--" => i.gateway = value,
            "IP4.DNS" => dns.push(value),
            _ => {}
        }
    }
    finish(&mut cur, &mut dns);
    out_list
}

/// A failed join, in a few words for a toast.
pub fn failure(output: &str, gave_password: bool) -> String {
    let secrets = [
        "Secrets were required",
        "No agents were available",
        "psk: property is invalid",
    ];
    if secrets.iter().any(|s| output.contains(s)) {
        return if gave_password {
            "Wrong password".into()
        } else {
            "Needs a password".into()
        };
    }
    if output.contains("Not authorized") || output.contains("not authorized") {
        return "Not allowed: polkit rule missing".into();
    }
    let line = output
        .lines()
        .find_map(|l| l.trim().strip_prefix("Error: "))
        .unwrap_or("Could not connect");
    let line = line
        .strip_prefix("Connection activation failed: ")
        .unwrap_or(line);
    line.trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaped_colons_stay_in_their_field() {
        assert_eq!(fields(r"*:72:WPA2:My\:Net"), ["*", "72", "WPA2", "My:Net"]);
        assert_eq!(fields(r"a\\b:c"), [r"a\b", "c"]);
    }

    #[test]
    fn the_list_keeps_one_row_per_name_with_the_one_in_use_first() {
        let out = " :40:WPA2:Office\n*:30:WPA1 WPA2:Home\n :80:WPA2:Office\n :55::Cafe\n :10:WPA2 802.1X:Corp\n :20::\n";
        let list = parse_list(out, &["Office".into()]);
        let names: Vec<&str> = list.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, ["Home", "Office", "Cafe", "Corp"]);
        assert!(list[0].active);
        assert_eq!(list[1].bars, 4, "the stronger of two sightings");
        assert!(list[1].known);
        assert!(!list[2].secure);
        assert!(list[3].enterprise);
    }

    #[test]
    fn devices_read_as_blocks() {
        let out = "GENERAL.DEVICE:wlan0\nGENERAL.TYPE:wifi\nGENERAL.STATE:100 (connected)\nGENERAL.CONNECTION:Mostly Harmless\nGENERAL.HWADDR:2C\\:CF\\:67\\:0A\\:41\\:9E\nIP4.ADDRESS[1]:192.168.0.13/24\nIP4.GATEWAY:192.168.0.1\nIP4.DNS[1]:1.1.1.1\nIP4.DNS[2]:9.9.9.9\n\nGENERAL.DEVICE:lo\nGENERAL.TYPE:loopback\n\nGENERAL.DEVICE:eth0\nGENERAL.TYPE:ethernet\nGENERAL.STATE:20 (unavailable)\nGENERAL.CONNECTION:--\nIP4.GATEWAY:--\n";
        let d = parse_devices(out);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].name, "wlan0");
        assert_eq!(d[0].state, "connected");
        assert_eq!(d[0].mac, "2c:cf:67:0a:41:9e");
        assert_eq!(d[0].dns, "1.1.1.1, 9.9.9.9");
        assert_eq!(d[1].state, "unavailable");
        assert!(d[1].connection.is_empty() && d[1].gateway.is_empty());
    }

    #[test]
    fn profiles_radio_and_failures() {
        assert_eq!(
            parse_profiles("Home:802-11-wireless\nWired:802-3-ethernet\nlo:loopback\n"),
            ["Home"]
        );
        assert_eq!(parse_radio("enabled\n"), Some(true));
        assert_eq!(
            failure("Error: Connection activation failed: Secrets were required, but not provided.", true),
            "Wrong password"
        );
        assert_eq!(
            failure("Error: Connection activation failed: No network with SSID 'x' found.", false),
            "No network with SSID 'x' found"
        );
    }
}
