//! nmcli's terse output read into Settings state. Only the parsing lives
//! here; the shell runs nmcli as jobs. Ported from pt35-menu.

use super::pages::{Iface, Network};
use gh0st_core::hotspot::{self, Security as ApSecurity};

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
/// NetworkManager sets to the network's name. The hotspot is not a network
/// to join or forget.
pub fn parse_profiles(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| {
            let f = fields(l);
            (f.get(1).map(String::as_str) == Some("802-11-wireless") && f[0] != hotspot::CONN)
                .then(|| f[0].clone())
        })
        .collect()
}

/// `nmcli -s -t -f 802-11-wireless.ssid,802-11-wireless-security.psk,
/// 802-11-wireless-security.key-mgmt connection show gh0st-hotspot`.
pub fn parse_hotspot(out: &str) -> (Option<String>, Option<String>, ApSecurity) {
    let (mut name, mut psk, mut security) = (None, None, ApSecurity::default());
    for line in out.lines() {
        let f = fields(line);
        let (Some(key), Some(v)) = (f.first(), f.get(1)) else {
            continue;
        };
        match key.as_str() {
            "802-11-wireless.ssid" if !v.is_empty() => name = Some(v.clone()),
            "802-11-wireless-security.psk" if !v.is_empty() => psk = Some(v.clone()),
            "802-11-wireless-security.key-mgmt" => security = ApSecurity::from_key_mgmt(v),
            _ => {}
        }
    }
    (name, psk, security)
}

/// `nmcli -t -f NAME,TYPE connection show`: every profile as (name, type).
pub fn parse_conns(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| {
            let f = fields(l);
            Some((f.first()?.clone(), f.get(1)?.clone()))
        })
        .filter(|(n, t)| (t == "802-11-wireless" && n != hotspot::CONN) || t == "802-3-ethernet")
        .collect()
}

/// `nmcli -t connection show NAME`: the settings Settings can change.
pub fn parse_conn(out: &str) -> super::pages::Conn {
    let mut c = super::pages::Conn::default();
    for line in out.lines() {
        let f = fields(line);
        let (Some(key), Some(v)) = (f.first(), f.get(1)) else {
            continue;
        };
        let v = v.clone();
        match key.as_str() {
            "connection.id" => c.name = v,
            "connection.type" => c.wifi = v == "802-11-wireless",
            "connection.autoconnect" => c.autoconnect = v == "yes",
            "ipv4.method" => c.manual = v == "manual",
            "ipv4.addresses" => c.address = v,
            "ipv4.gateway" if v != "--" => c.gateway = v,
            "ipv4.dns" => c.dns = v,
            "802-11-wireless.cloned-mac-address" | "802-3-ethernet.cloned-mac-address" => c.mac = v,
            "GENERAL.STATE" => c.active = v == "activated",
            "IP4.ADDRESS[1]" => c.current = v,
            _ => {}
        }
    }
    c
}

/// MAC policies, as nmcli spells them; empty is NetworkManager's default.
/// A network's address choices, in the order Left and Right step them.
/// `custom` is a fixed address the user types, kept as the address itself.
pub const MACS: &[(&str, &str)] = &[
    ("", "Default"),
    ("permanent", "Device"),
    ("stable", "Stable"),
    ("random", "Random"),
    (CUSTOM, "Custom"),
];

pub const CUSTOM: &str = "custom";

/// Where `v` sits in [`MACS`]: a fixed address is Custom.
pub fn mac_index(v: &str) -> usize {
    if valid_mac(v).is_some() {
        return MACS.len() - 1;
    }
    MACS.iter().position(|(k, _)| *k == v).unwrap_or(0)
}

pub fn mac_label(v: &str) -> String {
    if let Some(m) = valid_mac(v) {
        return m;
    }
    MACS.iter()
        .find(|(k, _)| *k == v)
        .map_or("Default", |(_, l)| l)
        .to_string()
}

/// A typed address, as `AA:BB:CC:DD:EE:FF`: six hex pairs split by `:` or
/// `-`. Not a group address and not all zeros, which no card may use.
pub fn valid_mac(s: &str) -> Option<String> {
    let parts: Vec<&str> = s.trim().split([':', '-']).collect();
    if parts.len() != 6 || parts.iter().any(|p| p.len() != 2) {
        return None;
    }
    let bytes: Vec<u8> = parts
        .iter()
        .map(|p| u8::from_str_radix(p, 16).ok())
        .collect::<Option<_>>()?;
    if bytes[0] & 1 == 1 || bytes.iter().all(|&b| b == 0) {
        return None;
    }
    Some(
        bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// A random address to start from: locally administered and unicast, so it
/// can't clash with any card's own.
pub fn random_mac() -> String {
    let mut b = [0u8; 6];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| {
        use std::io::Read;
        f.read_exact(&mut b)
    });
    if read.is_err() {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        b.copy_from_slice(&t.to_le_bytes()[..6]);
    }
    b[0] = (b[0] | 0x02) & !0x01;
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Checks a typed IPv4 address with optional prefix, or a list of them.
pub fn valid_ipv4(s: &str, prefix: bool) -> bool {
    let (ip, len) = match s.split_once('/') {
        Some((ip, len)) if prefix => (ip, Some(len)),
        Some(_) => return false,
        None => (s, None),
    };
    let ok_ip = ip.split('.').count() == 4 && ip.split('.').all(|p| p.parse::<u8>().is_ok());
    ok_ip && len.is_none_or(|l| l.parse::<u8>().is_ok_and(|n| n <= 32))
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
/// GENERAL.HWADDR,IP4.ADDRESS,IP4.GATEWAY,IP4.DNS,IP6.ADDRESS device show`: blocks of
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
            "GENERAL.TYPE" => {
                i.kind = value.clone();
                *kind = value;
            }
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
            "IP6.ADDRESS" if !value.to_lowercase().starts_with("fe80") => i.ipv6.push(value),
            _ => {}
        }
    }
    finish(&mut cur, &mut dns);
    out_list
}

/// A cable's link speed in Mb/s, from sysfs. None for Wi-Fi, a link
/// that is down, or a driver that doesn't say.
pub fn speed(sys: &std::path::Path, iface: &str) -> Option<u32> {
    let s = std::fs::read_to_string(sys.join(iface).join("speed")).ok()?;
    s.trim()
        .parse::<i64>()
        .ok()
        .filter(|&n| n > 0)
        .map(|n| n as u32)
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
        .unwrap_or("Couldn't connect");
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
        assert_eq!(d[0].kind, "wifi");
    }

    #[test]
    fn ipv6_keeps_routable_addresses() {
        let out = "GENERAL.DEVICE:eth0\nGENERAL.TYPE:ethernet\nIP6.ADDRESS[1]:fd56\\:18a6\\:\\:ecf/128\nIP6.ADDRESS[2]:fe80\\:\\:da1a/64\n";
        let d = parse_devices(out);
        assert_eq!(d[0].ipv6, ["fd56:18a6::ecf/128"]);
    }

    #[test]
    fn a_connection_reads_its_settings() {
        let out = "connection.id:Mostly Harmless\nconnection.type:802-11-wireless\nconnection.autoconnect:yes\n802-11-wireless.cloned-mac-address:stable\nipv4.method:manual\nipv4.dns:1.1.1.1,9.9.9.9\nipv4.addresses:192.168.0.50/24\nipv4.gateway:192.168.0.1\nGENERAL.STATE:activated\nIP4.ADDRESS[1]:192.168.0.50/24\n";
        let c = parse_conn(out);
        assert!(c.wifi && c.autoconnect && c.manual && c.active);
        assert_eq!(c.address, "192.168.0.50/24");
        assert_eq!(c.dns, "1.1.1.1,9.9.9.9");
        assert_eq!(mac_label(&c.mac), "Stable");
    }

    #[test]
    fn typed_addresses() {
        assert_eq!(
            valid_mac("02-1a-2b-3c-4d-5e").as_deref(),
            Some("02:1A:2B:3C:4D:5E")
        );
        assert_eq!(
            valid_mac(" 02:1A:2B:3C:4D:5E ").as_deref(),
            Some("02:1A:2B:3C:4D:5E")
        );
        for bad in [
            "",
            "02:1A:2B:3C:4D",
            "01:00:5E:00:00:01",
            "00:00:00:00:00:00",
            "02:1A:2B:3C:4D:ZZ",
            "021A2B3C4D5E",
        ] {
            assert_eq!(valid_mac(bad), None, "{bad}");
        }
        let r = random_mac();
        let first = u8::from_str_radix(&r[..2], 16).unwrap();
        assert!(valid_mac(&r).is_some() && first & 2 == 2 && first & 1 == 0);
        assert_eq!(mac_label("02:1a:2b:3c:4d:5e"), "02:1A:2B:3C:4D:5E");
        assert_eq!(mac_index("02:1a:2b:3c:4d:5e"), MACS.len() - 1);
        assert_eq!(mac_index("random"), 3);
        assert_eq!(
            parse_conns(
                "Home:802-11-wireless:wlan0\nlo:loopback:lo\nWired connection 1:802-3-ethernet:\n"
            ),
            [
                ("Home".to_string(), "802-11-wireless".to_string()),
                (
                    "Wired connection 1".to_string(),
                    "802-3-ethernet".to_string()
                )
            ]
        );
    }

    #[test]
    fn typed_addresses_are_checked() {
        assert!(valid_ipv4("192.168.0.50/24", true));
        assert!(!valid_ipv4("192.168.0.50/33", true));
        assert!(!valid_ipv4("192.168.0/24", true));
        assert!(valid_ipv4("192.168.0.1", false));
        assert!(!valid_ipv4("192.168.0.1/24", false));
    }

    #[test]
    fn the_hotspot_is_no_saved_network() {
        let out = "Home:802-11-wireless\ngh0st-hotspot:802-11-wireless\nWired:802-3-ethernet\n";
        assert_eq!(parse_profiles(out), ["Home"]);
        let conns: Vec<String> = parse_conns(out).into_iter().map(|(n, _)| n).collect();
        assert_eq!(conns, ["Home", "Wired"]);
    }

    #[test]
    fn the_hotspot_reads_its_name_password_and_security() {
        let out = "802-11-wireless.ssid:zero-abcd\n802-11-wireless-security.psk:a\\:b\n\
                   802-11-wireless-security.key-mgmt:sae\n";
        let (name, psk, sec) = parse_hotspot(out);
        assert_eq!(name.as_deref(), Some("zero-abcd"));
        assert_eq!(psk.as_deref(), Some("a:b"));
        assert_eq!(sec, ApSecurity::Wpa3);
        let none = parse_hotspot("");
        assert!(none.0.is_none() && none.1.is_none());
        assert_eq!(
            parse_hotspot("802-11-wireless-security.key-mgmt:wpa-psk\n").2,
            ApSecurity::Mixed
        );
    }

    #[test]
    fn profiles_radio_and_failures() {
        assert_eq!(
            parse_profiles("Home:802-11-wireless\nWired:802-3-ethernet\nlo:loopback\n"),
            ["Home"]
        );
        assert_eq!(parse_radio("enabled\n"), Some(true));
        assert_eq!(
            failure(
                "Error: Connection activation failed: Secrets were required, but not provided.",
                true
            ),
            "Wrong password"
        );
        assert_eq!(
            failure(
                "Error: Connection activation failed: No network with SSID 'x' found.",
                false
            ),
            "No network with SSID 'x' found"
        );
    }
}
