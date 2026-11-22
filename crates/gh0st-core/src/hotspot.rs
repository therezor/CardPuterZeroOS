//! The hotspot: an access point on wlan0 in place of the Wi-Fi connection.
//! NetworkManager runs it in shared mode, which brings DHCP and DNS
//! (dnsmasq) and NAT out a cable (nftables). The root helper is the only
//! writer of its keyfile; Settings reads it back through nmcli and counts
//! clients with iw. docs/hotspot.md has the why.

use std::io::Read;

/// The NetworkManager profile, by name.
pub const CONN: &str = "gh0st-hotspot";
pub const IFACE: &str = "wlan0";
/// Root only, mode 0600: it holds the password.
pub const KEYFILE: &str = "/etc/NetworkManager/system-connections/gh0st-hotspot.nmconnection";
/// Shared mode gives the device this address.
pub const ADDRESS: &str = "10.42.0.1";
pub const NAME_MAX: usize = 32;
pub const PASSWORD_MIN: usize = 8;
pub const PASSWORD_MAX: usize = 63;
const PASSWORD_LEN: usize = 12;
/// Lowercase only, no l or o: quick to type on any keyboard, 4.6 bits each.
const LETTERS: &[u8] = b"abcdefghijkmnpqrstuvwxyz";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Security {
    /// SAE with protected management frames required.
    #[default]
    Wpa3,
    /// WPA2 for older devices, WPA3 for the rest.
    Mixed,
}

impl Security {
    pub const ALL: [Security; 2] = [Security::Wpa3, Security::Mixed];

    pub fn verb(self) -> &'static str {
        match self {
            Security::Wpa3 => "wpa3",
            Security::Mixed => "mixed",
        }
    }

    pub fn from_verb(s: &str) -> Option<Security> {
        Security::ALL.into_iter().find(|v| v.verb() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            Security::Wpa3 => "WPA3",
            Security::Mixed => "WPA2/WPA3",
        }
    }

    /// From `802-11-wireless-security.key-mgmt`.
    pub fn from_key_mgmt(s: &str) -> Security {
        if s == "sae" {
            Security::Wpa3
        } else {
            Security::Mixed
        }
    }

    /// Key management and PMF (2 optional, 3 required). With `wpa-psk` and
    /// PMF optional, NetworkManager offers SAE as well.
    fn keyfile(self) -> (&'static str, u8) {
        match self {
            Security::Wpa3 => ("sae", 3),
            Security::Mixed => ("wpa-psk", 2),
        }
    }
}

pub struct Profile {
    pub uuid: String,
    pub name: String,
    pub psk: String,
    pub security: Security,
}

impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Profile")
            .field("name", &self.name)
            .field("security", &self.security)
            .finish_non_exhaustive()
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        // SAFETY: zeroes keep the string valid UTF-8.
        crate::password::wipe(unsafe { self.psk.as_bytes_mut() });
    }
}

impl Profile {
    /// A fresh profile: a neutral name and a random password.
    pub fn new() -> std::io::Result<Profile> {
        Ok(Profile {
            uuid: new_uuid()?,
            name: default_name()?,
            psk: new_password()?,
            security: Security::default(),
        })
    }

    /// The keyfile. Off after a boot, bound to wlan0, the SSID as bytes so
    /// no name needs escaping.
    pub fn render(&self) -> String {
        let ssid: String = self.name.bytes().map(|b| format!("{b};")).collect();
        let (key_mgmt, pmf) = self.security.keyfile();
        format!(
            "[connection]\nid={CONN}\nuuid={}\ntype=wifi\ninterface-name={IFACE}\nautoconnect=false\n\n\
             [wifi]\nmode=ap\nband=bg\nssid={ssid}\ncloned-mac-address=stable\nap-isolation=1\n\n\
             [wifi-security]\nkey-mgmt={key_mgmt}\npmf={pmf}\nproto=rsn\npairwise=ccmp\ngroup=ccmp\npsk={}\n\n\
             [ipv4]\nmethod=shared\n\n[ipv6]\nmethod=disabled\n",
            self.uuid,
            escape(&self.psk),
        )
    }

    /// Reads a keyfile back, ours or one NetworkManager rewrote. `None`
    /// when a field is missing or invalid.
    pub fn parse(text: &str) -> Option<Profile> {
        let (mut uuid, mut name, mut psk, mut key_mgmt) = (None, None, None, None);
        let mut section = "";
        for line in text.lines() {
            if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = s;
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match (section, k) {
                ("connection", "uuid") => uuid = Some(v.to_string()),
                ("wifi", "ssid") => name = ssid(v),
                ("wifi-security", "psk") => psk = Some(unescape(v)),
                ("wifi-security", "key-mgmt") => key_mgmt = Some(v.to_string()),
                _ => {}
            }
        }
        let p = Profile {
            uuid: uuid?,
            name: name?,
            psk: psk?,
            security: Security::from_key_mgmt(&key_mgmt?),
        };
        (valid_name(&p.name) && valid_password(&p.psk)).then_some(p)
    }
}

/// An SSID as a byte list (`104;105;`) or a plain string.
fn ssid(v: &str) -> Option<String> {
    let list = v.contains(';')
        && v.split(';')
            .all(|b| b.is_empty() || b.parse::<u8>().is_ok());
    if !list {
        return Some(unescape(v));
    }
    let bytes: Vec<u8> = v
        .split(';')
        .filter(|b| !b.is_empty())
        .map(|b| b.parse().ok())
        .collect::<Option<_>>()?;
    String::from_utf8(bytes).ok()
}

/// GKeyFile's escapes for a string value.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            ' ' if i == 0 => out.push_str("\\s"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(c) => out.push(c),
            None => {}
        }
    }
    out
}

/// 1 to 32 bytes, no control characters, no space at either end.
pub fn valid_name(s: &str) -> bool {
    (1..=NAME_MAX).contains(&s.len()) && !s.chars().any(char::is_control) && s.trim() == s
}

/// 8 to 63 printable ASCII characters without spaces: what WPA takes, less
/// the space, which is easy to get wrong when typing it elsewhere.
pub fn valid_password(s: &str) -> bool {
    (PASSWORD_MIN..=PASSWORD_MAX).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_graphic())
}

/// Why a name or password is refused, for a dialog.
pub fn name_problem(s: &str) -> Option<&'static str> {
    if s.is_empty() {
        Some("Type a name")
    } else if s.len() > NAME_MAX {
        Some("Too long: 32 bytes at most")
    } else if !valid_name(s) {
        Some("No spaces at the ends")
    } else {
        None
    }
}

pub fn password_problem(s: &str) -> Option<&'static str> {
    if s.len() < PASSWORD_MIN {
        Some("Too short: 8 or more")
    } else if s.len() > PASSWORD_MAX {
        Some("Too long: 63 at most")
    } else if !valid_password(s) {
        Some("Letters, digits and symbols, no spaces")
    } else {
        None
    }
}

fn random(n: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf)
}

/// `n` letters, by rejection so each is equally likely.
fn letters(n: usize) -> std::io::Result<String> {
    let limit = (256 / LETTERS.len() * LETTERS.len()) as u8;
    let mut out = String::with_capacity(n);
    while out.len() < n {
        for b in random(n * 2)? {
            if b < limit && out.len() < n {
                out.push(LETTERS[b as usize % LETTERS.len()] as char);
            }
        }
    }
    Ok(out)
}

/// 12 letters: about 55 bits, out of reach of an offline guess.
pub fn new_password() -> std::io::Result<String> {
    letters(PASSWORD_LEN)
}

/// Neutral: never the device's or the user's name.
pub fn default_name() -> std::io::Result<String> {
    Ok(format!("zero-{}", letters(4)?))
}

fn new_uuid() -> std::io::Result<String> {
    let mut b = random(16)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

/// `iw dev wlan0 station dump`: one block per client.
pub fn stations(dump: &str) -> usize {
    dump.lines().filter(|l| l.starts_with("Station ")).count()
}

/// What a phone camera reads to join: the common `WIFI:` form. WPA3-only
/// says SAE, so Android saves it as WPA3.
pub fn wifi_qr(name: &str, psk: &str, security: Security) -> String {
    let esc = |s: &str| {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            if matches!(c, '\\' | ';' | ',' | ':' | '"') {
                out.push('\\');
            }
            out.push(c);
        }
        out
    };
    let kind = match security {
        Security::Wpa3 => "SAE",
        Security::Mixed => "WPA",
    };
    format!("WIFI:T:{kind};S:{};P:{};;", esc(name), esc(psk))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, psk: &str, security: Security) -> Profile {
        Profile {
            uuid: "3599de69-63e1-4917-9906-6069d0d6c83a".into(),
            name: name.into(),
            psk: psk.into(),
            security,
        }
    }

    #[test]
    fn a_profile_reads_back_what_it_wrote() {
        for (name, psk) in [
            ("zero-abcd", "kfwmtqzrapdh"),
            ("Café; a=b", r"back\slash#;="),
            ("Галя_WiFi", "~!@#$%^&*()_+"),
        ] {
            for sec in Security::ALL {
                let text = profile(name, psk, sec).render();
                let p = Profile::parse(&text).expect("parses");
                assert_eq!(
                    (p.name.as_str(), p.psk.as_str(), p.security),
                    (name, psk, sec)
                );
            }
        }
    }

    #[test]
    fn the_keyfile_has_the_fixed_lines() {
        let t = profile("zero-abcd", "kfwmtqzrapdh", Security::Wpa3).render();
        for line in [
            "id=gh0st-hotspot",
            "interface-name=wlan0",
            "autoconnect=false",
            "mode=ap",
            "ssid=122;101;114;111;45;97;98;99;100;",
            "cloned-mac-address=stable",
            "ap-isolation=1",
            "key-mgmt=sae",
            "pmf=3",
            "proto=rsn",
            "pairwise=ccmp",
            "group=ccmp",
            "method=shared",
            "method=disabled",
        ] {
            assert!(t.lines().any(|l| l == line), "{line}");
        }
        let m = profile("zero-abcd", "kfwmtqzrapdh", Security::Mixed).render();
        assert!(m.contains("key-mgmt=wpa-psk\npmf=2\n"));
        assert!(
            profile("x", r"a\b", Security::Wpa3)
                .render()
                .contains("psk=a\\\\b\n")
        );
    }

    #[test]
    fn a_keyfile_networkmanager_rewrote_still_reads() {
        let text = "[connection]\nid=gh0st-hotspot\nuuid=u-1\n\n[wifi]\nssid=zero-abcd\n\n\
                    [wifi-security]\nkey-mgmt=wpa-psk\npsk=kfwmtqzrapdh\n";
        let p = Profile::parse(text).unwrap();
        assert_eq!(p.name, "zero-abcd");
        assert_eq!(p.security, Security::Mixed);
        assert!(Profile::parse("[wifi]\nssid=x\n").is_none());
        assert!(Profile::parse(&text.replace("kfwmtqzrapdh", "short")).is_none());
    }

    #[test]
    fn names_and_passwords_at_their_edges() {
        assert!(!valid_name(""));
        assert!(valid_name("a"));
        assert!(valid_name(&"a".repeat(32)));
        assert!(!valid_name(&"a".repeat(33)));
        assert!(!valid_name(&"я".repeat(17)), "34 bytes");
        assert!(!valid_name("new\nline"));
        assert!(!valid_name(" lead"));
        assert!(valid_name("in side"));
        assert!(!valid_password(&"a".repeat(7)));
        assert!(valid_password(&"a".repeat(8)));
        assert!(valid_password(&"a".repeat(63)));
        assert!(!valid_password(&"a".repeat(64)));
        assert!(!valid_password("with space"));
        assert!(!valid_password("pässwörd"));
        assert!(!valid_password("tab\there!"));
        assert_eq!(password_problem("1234567"), Some("Too short: 8 or more"));
        assert_eq!(name_problem(""), Some("Type a name"));
        assert_eq!(name_problem("zero"), None);
    }

    #[test]
    fn new_passwords_are_twelve_plain_letters() {
        let a = new_password().unwrap();
        let b = new_password().unwrap();
        assert_eq!(a.len(), 12);
        assert!(a.bytes().all(|c| LETTERS.contains(&c)));
        assert!(!a.contains('l') && !a.contains('o'));
        assert_ne!(a, b);
        assert!(valid_password(&a));
        let n = default_name().unwrap();
        assert!(n.starts_with("zero-") && n.len() == 9 && valid_name(&n));
        let p = Profile::new().unwrap();
        assert_eq!(p.uuid.len(), 36);
        assert_eq!(&p.uuid[14..15], "4");
        assert!(
            !format!("{p:?}").contains(&p.psk),
            "Debug hides the password"
        );
    }

    #[test]
    fn stations_count_blocks() {
        assert_eq!(stations(""), 0);
        let one =
            "Station ee:d6:24:aa:c5:6d (on wlan0)\n\tinactive time:\t9000 ms\n\tauthorized:\tyes\n";
        assert_eq!(stations(one), 1);
        assert_eq!(stations(&format!("{one}{}", one.replace("ee:", "92:"))), 2);
    }

    #[test]
    fn the_qr_text_escapes_its_separators() {
        assert_eq!(
            wifi_qr("a;b:c", r#"p\w,"x""#, Security::Mixed),
            r#"WIFI:T:WPA;S:a\;b\:c;P:p\\w\,\"x\";;"#
        );
        assert!(wifi_qr("zero", "kfwmtqzrapdh", Security::Wpa3).starts_with("WIFI:T:SAE;"));
    }
}
