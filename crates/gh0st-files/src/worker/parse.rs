//! What servers and tools print, read into entries: `ls -l` lines (FTP
//! LIST, `bsdtar -tv`), FTP MLSD facts, and WebDAV PROPFIND replies.

use crate::model::{Entry, Kind};

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// Seconds since 1970 for a UTC date.
pub fn epoch(y: i64, m: u32, d: u32, hh: u32, mm: u32, ss: u32) -> i64 {
    // Days from civil, after Howard Hinnant.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + (hh * 3600 + mm * 60 + ss) as i64
}

fn month(s: &str) -> Option<u32> {
    let s = s.to_ascii_lowercase();
    MONTHS
        .iter()
        .position(|m| s.starts_with(m))
        .map(|i| i as u32 + 1)
}

fn this_year() -> i64 {
    // SAFETY: time and gmtime_r into a local struct tm.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&t, &mut tm);
        tm.tm_year as i64 + 1900
    }
}

/// One `ls -l` line: `drwxr-xr-x 2 me me 4096 Nov 24 13:09 name`, or with
/// a year in place of the time. A symlink's name ends ` -> target`. The
/// name keeps any slashes, as an archive's paths have.
pub fn ls_line(line: &str) -> Option<Entry> {
    let mut rest = line.trim_end_matches(['\r', '\n']);
    let mut fields = Vec::with_capacity(8);
    for _ in 0..8 {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace)?;
        fields.push(&rest[..end]);
        rest = &rest[end..];
    }
    // One space after the date, then the name as it is.
    let name = rest.strip_prefix(' ').unwrap_or(rest);
    let mode = fields[0];
    if mode.len() < 10 {
        return None;
    }
    let size: u64 = fields[4].parse().ok()?;
    // "Nov 24", or "24 Nov" where the locale puts the day first.
    let (mon, day) = match (month(fields[5]), fields[6].parse::<u32>()) {
        (Some(m), Ok(d)) => (m, d),
        _ => (month(fields[6])?, fields[5].parse().ok()?),
    };
    let (year, hh, mm) = match fields[7].split_once(':') {
        Some((h, m)) => (this_year(), h.parse().ok()?, m.parse().ok()?),
        None => (fields[7].parse().ok()?, 0, 0),
    };
    let perm = mode_bits(&mode[1..10]);
    let (kind, name, link) = match mode.as_bytes()[0] {
        b'd' => (Kind::Dir, name.trim_end_matches('/'), None),
        b'l' => match name.split_once(" -> ") {
            Some((n, t)) => (Kind::Link, n, Some(t.to_string())),
            None => (Kind::Link, name, None),
        },
        b'-' => (Kind::File, name, None),
        _ => (Kind::Other, name, None),
    };
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(Entry {
        name: name.into(),
        kind,
        size: if kind == Kind::Dir { 0 } else { size },
        mtime: epoch(year, mon, day, hh, mm, 0),
        mode: perm,
        link,
    })
}

fn mode_bits(rwx: &str) -> u32 {
    rwx.bytes()
        .enumerate()
        .filter(|(_, c)| !matches!(c, b'-' | b'S' | b'T'))
        .fold(0, |m, (i, _)| m | (1 << (8 - i)))
}

/// One MLSD line: `type=file;size=12;modify=20251124130900; name`.
pub fn mlsd_line(line: &str) -> Option<Entry> {
    let line = line.trim_end_matches(['\r', '\n']);
    let (facts, name) = line.split_once(' ')?;
    let mut e = Entry::file(name, 0);
    e.mode = 0;
    for f in facts.split(';').filter(|f| !f.is_empty()) {
        let (k, v) = f.split_once('=')?;
        match k.to_ascii_lowercase().as_str() {
            "type" => {
                e.kind = match v.to_ascii_lowercase().as_str() {
                    "file" => Kind::File,
                    "dir" => Kind::Dir,
                    "cdir" | "pdir" => return None,
                    t if t.contains("slink") || t.contains("symlink") => Kind::Link,
                    _ => Kind::Other,
                }
            }
            "size" => e.size = v.parse().unwrap_or(0),
            "modify" if v.len() >= 14 => {
                let n = |a: usize, b: usize| v.get(a..b).and_then(|x| x.parse::<u32>().ok());
                e.mtime = epoch(
                    n(0, 4)? as i64,
                    n(4, 6)?,
                    n(6, 8)?,
                    n(8, 10)?,
                    n(10, 12)?,
                    n(12, 14)?,
                );
            }
            "unix.mode" => e.mode = u32::from_str_radix(v.trim_start_matches("0o"), 8).unwrap_or(0),
            _ => {}
        }
    }
    if e.kind == Kind::Dir {
        e.size = 0;
    }
    (!e.name.is_empty()).then_some(e)
}

/// "Tue, 24 Nov 2026 13:09:00 GMT".
pub fn http_date(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let [_, d, m, y, t, ..] = parts[..] else {
        return None;
    };
    let mut hms = t.split(':').map(|x| x.parse::<u32>().ok());
    Some(epoch(
        y.parse().ok()?,
        month(m)?,
        d.parse().ok()?,
        hms.next()??,
        hms.next()??,
        hms.next()??,
    ))
}

/// `%20` and friends.
pub fn unpct(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Every byte but letters, digits, `-._~` and `/` as `%XX`.
pub fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn unxml(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';') else {
            break;
        };
        let ent = &rest[1..end];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16)
                .ok()
                .and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The tags of an XML reply, namespace prefixes dropped: (name, closing,
/// self-closing, text after it).
fn tags(xml: &str) -> Vec<(String, bool, bool, &str)> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let Some(end) = rest.find('>') else {
            break;
        };
        let inner = &rest[..end];
        rest = &rest[end + 1..];
        if inner.starts_with('?') || inner.starts_with('!') {
            continue;
        }
        let closing = inner.starts_with('/');
        let selfc = inner.ends_with('/');
        let name = inner
            .trim_start_matches('/')
            .trim_end_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase();
        let text = &rest[..rest.find('<').unwrap_or(rest.len())];
        out.push((local, closing, selfc, text));
    }
    out
}

/// A PROPFIND reply's responses: each one's href path, decoded, and its
/// entry, named by the href's last part.
pub fn propfind(xml: &str) -> Vec<(String, Entry)> {
    let mut out = Vec::new();
    let mut cur: Option<(String, Entry)> = None;
    for (name, closing, selfc, text) in tags(xml) {
        match (name.as_str(), closing) {
            ("response", false) if !selfc => cur = Some((String::new(), Entry::file("", 0))),
            ("response", true) => {
                if let Some((href, mut e)) = cur.take() {
                    let path = href_path(&href);
                    e.name = path
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    out.push((path, e));
                }
            }
            ("href", false) => {
                if let Some(c) = cur.as_mut() {
                    c.0 = unxml(text.trim());
                }
            }
            ("collection", false) => {
                if let Some(c) = cur.as_mut() {
                    c.1.kind = Kind::Dir;
                }
            }
            ("getcontentlength", false) => {
                if let Some(c) = cur.as_mut() {
                    c.1.size = text.trim().parse().unwrap_or(0);
                }
            }
            ("getlastmodified", false) => {
                if let Some(c) = cur.as_mut() {
                    c.1.mtime = http_date(&unxml(text.trim())).unwrap_or(0);
                }
            }
            _ => {}
        }
    }
    for (_, e) in &mut out {
        e.mode = 0;
        if e.kind == Kind::Dir {
            e.size = 0;
        }
    }
    out
}

/// An href's path: a full URL loses its scheme and host, and `%XX` decode.
pub fn href_path(href: &str) -> String {
    let path = match href.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("/", |i| &rest[i..]),
        None => href,
    };
    unpct(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls_lines_read_names_with_spaces_links_and_years() {
        let e =
            ls_line("-rw-r--r--    1 1000     1000       214 Nov 24  2025 my notes.txt").unwrap();
        assert_eq!(
            (e.name.as_str(), e.kind, e.size, e.mode),
            ("my notes.txt", Kind::File, 214, 0o644)
        );
        assert_eq!(e.mtime, epoch(2025, 11, 24, 0, 0, 0));
        let l = ls_line("lrwxrwxrwx 1 me me 7 Jan  2 10:30 latest -> v1.2/a b").unwrap();
        assert_eq!(
            (l.name.as_str(), l.link.as_deref()),
            ("latest", Some("v1.2/a b"))
        );
        let d = ls_line("drwxr-xr-x  0 user   group       0 Nov 24 13:09 docs/sub/").unwrap();
        assert_eq!(
            (d.name.as_str(), d.kind, d.size),
            ("docs/sub", Kind::Dir, 0)
        );
        assert!(ls_line("total 12").is_none());
        let dm = ls_line("-rw-r--r--  0 501    0       2 24 Nov 13:43 d/a.txt").unwrap();
        assert_eq!((dm.name.as_str(), dm.size), ("d/a.txt", 2));
        assert!(ls_line("drwxr-xr-x 2 me me 4096 Nov 24 13:09 .").is_none());
        let u =
            ls_line("-rw-r--r-- 1 me me 9 Feb 29 2024 \u{0444}\u{0430}\u{0439}\u{043b}").unwrap();
        assert_eq!(u.name, "\u{0444}\u{0430}\u{0439}\u{043b}");
    }

    #[test]
    fn mlsd_lines_read_facts() {
        let e =
            mlsd_line("type=file;size=1240;modify=20251124130900;UNIX.mode=0644; lyrics v2.txt")
                .unwrap();
        assert_eq!(
            (e.name.as_str(), e.size, e.mode),
            ("lyrics v2.txt", 1240, 0o644)
        );
        assert_eq!(e.mtime, epoch(2025, 11, 24, 13, 9, 0));
        assert_eq!(
            mlsd_line("type=dir;modify=20250101000000; scans")
                .unwrap()
                .kind,
            Kind::Dir
        );
        assert!(mlsd_line("type=cdir; .").is_none());
        assert_eq!(
            mlsd_line("type=OS.unix=symlink; l").unwrap().kind,
            Kind::Link
        );
    }

    #[test]
    fn epochs_and_http_dates() {
        assert_eq!(epoch(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(epoch(2000, 3, 1, 0, 0, 0), 951_868_800);
        assert_eq!(
            http_date("Tue, 24 Nov 2026 13:09:00 GMT"),
            Some(epoch(2026, 11, 24, 13, 9, 0))
        );
    }

    #[test]
    fn propfind_reads_any_prefix() {
        let apache = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
<D:response><D:href>/dav/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>
<D:response><D:href>/dav/my%20notes.txt</D:href><D:propstat><D:prop><D:resourcetype/><D:getcontentlength>12</D:getcontentlength><D:getlastmodified>Tue, 24 Nov 2026 13:09:00 GMT</D:getlastmodified></D:prop></D:propstat></D:response>
<D:response><D:href>/dav/Fotos%20%26%20more/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat></D:response>
</D:multistatus>"#;
        let v = propfind(apache);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].0, "/dav/");
        assert_eq!((v[1].1.name.as_str(), v[1].1.size), ("my notes.txt", 12));
        assert_eq!(
            (v[2].1.name.as_str(), v[2].1.kind),
            ("Fotos & more", Kind::Dir)
        );
        let nextcloud = r#"<d:multistatus xmlns:d="DAV:" xmlns:oc="http://owncloud.org/ns"><d:response><d:href>https://cloud.example/remote.php/dav/files/me/a&amp;b.txt</d:href><d:propstat><d:prop><d:getcontentlength>5</d:getcontentlength><d:resourcetype/></d:prop></d:propstat></d:response></d:multistatus>"#;
        let v = propfind(nextcloud);
        assert_eq!(v[0].0, "/remote.php/dav/files/me/a&b.txt");
        assert_eq!(v[0].1.name, "a&b.txt");
        let bare = r#"<multistatus xmlns="DAV:"><response><href>/x/%E2%9C%93.txt</href><propstat><prop><getcontentlength>1</getcontentlength></prop></propstat></response></multistatus>"#;
        assert_eq!(propfind(bare)[0].1.name, "\u{2713}.txt");
    }

    #[test]
    fn percent_round_trips() {
        let s = "/a b/\u{2713}#?%.txt";
        assert_eq!(unpct(&pct(s)), s);
        assert_eq!(pct("/a b"), "/a%20b");
    }
}
