//! Key and value files: INI and its kin (systemd units, desktop files),
//! TOML and YAML.

use super::{Out, Role};

/// A value after `=` or `:`: numbers, booleans, quoted strings.
fn value(line: &[u8], from: usize, o: &mut Out) {
    let v = line[from..].trim_ascii();
    let start = from + (line[from..].len() - line[from..].trim_ascii_start().len());
    let end = start + v.len();
    o.push(start, Role::Plain);
    let role = if v.first().is_some_and(|c| *c == b'"' || *c == b'\'') {
        Role::String
    } else if matches!(
        v,
        b"true" | b"false" | b"yes" | b"no" | b"on" | b"off" | b"null" | b"~" | b"True" | b"False"
    ) || v
        .iter()
        .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'+' | b'_' | b':'))
        && !v.is_empty()
    {
        Role::Number
    } else {
        Role::Plain
    };
    o.push(end, role);
    o.push(line.len(), Role::Plain);
}

pub(super) fn ini(line: &[u8], o: &mut Out) -> u8 {
    let t = line.trim_ascii_start();
    let lead = line.len() - t.len();
    if t.starts_with(b"#") || t.starts_with(b";") {
        o.push(line.len(), Role::Comment);
    } else if t.starts_with(b"[") {
        o.push(line.len(), Role::Heading);
    } else if let Some(eq) = line.iter().position(|&c| c == b'=') {
        o.push(lead, Role::Plain);
        o.push(eq, Role::Func);
        o.push(eq + 1, Role::Punct);
        value(line, eq + 1, o);
    } else {
        o.push(line.len(), Role::Plain);
    }
    0
}

/// State 1 and 2: inside a `"""` or `'''` string that runs on.
pub(super) fn toml(state: u8, line: &[u8], o: &mut Out) -> u8 {
    if state == 1 || state == 2 {
        let close: &[u8] = if state == 1 { b"\"\"\"" } else { b"'''" };
        return match line.windows(3).position(|w| w == close) {
            Some(p) => {
                o.push(p + 3, Role::String);
                o.push(line.len(), Role::Plain);
                0
            }
            None => {
                o.push(line.len(), Role::String);
                state
            }
        };
    }
    let t = line.trim_ascii_start();
    let lead = line.len() - t.len();
    if t.starts_with(b"#") {
        o.push(line.len(), Role::Comment);
        return 0;
    }
    if t.starts_with(b"[") {
        o.push(line.len(), Role::Heading);
        return 0;
    }
    let Some(eq) = line.iter().position(|&c| c == b'=') else {
        o.push(line.len(), Role::Plain);
        return 0;
    };
    o.push(lead, Role::Plain);
    o.push(eq, Role::Func);
    o.push(eq + 1, Role::Punct);
    let rest = &line[eq + 1..];
    for (open, st) in [(&b"\"\"\""[..], 1u8), (&b"'''"[..], 2u8)] {
        if let Some(p) = rest.windows(3).position(|w| w == open) {
            let after = &rest[p + 3..];
            if !after.windows(3).any(|w| w == open) {
                o.push(eq + 1 + p, Role::Plain);
                o.push(line.len(), Role::String);
                return st;
            }
        }
    }
    // A comment after the value.
    let hash = comment_at(rest);
    let vend = hash.map_or(line.len(), |h| eq + 1 + h);
    value(&line[..vend], eq + 1, o);
    if let Some(h) = hash {
        o.push(eq + 1 + h, Role::Plain);
        o.push(line.len(), Role::Comment);
    }
    0
}

/// Where `#` starts a comment: outside quotes, after a blank.
fn comment_at(s: &[u8]) -> Option<usize> {
    let mut quote = None;
    for (i, &c) in s.iter().enumerate() {
        match (quote, c) {
            (None, b'"' | b'\'') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, b'#') if i == 0 || s[i - 1].is_ascii_whitespace() => return Some(i),
            _ => {}
        }
    }
    None
}

pub(super) fn yaml(line: &[u8], o: &mut Out) -> u8 {
    let t = line.trim_ascii_start();
    let lead = line.len() - t.len();
    if t.starts_with(b"#") {
        o.push(line.len(), Role::Comment);
        return 0;
    }
    if t.starts_with(b"---") || t.starts_with(b"...") {
        o.push(line.len(), Role::Heading);
        return 0;
    }
    let mut i = lead;
    while line[i..].starts_with(b"- ") {
        o.push(i + 1, Role::Punct);
        i += 2;
    }
    let hash = comment_at(&line[i..]).map(|h| i + h);
    let body_end = hash.unwrap_or(line.len());
    // key: value, the key outside quotes.
    let colon = line[i..body_end]
        .windows(2)
        .position(|w| w[0] == b':' && w[1] == b' ')
        .map(|p| i + p)
        .or_else(|| (body_end > i && line[body_end - 1] == b':').then(|| body_end - 1));
    match colon {
        Some(c) if !line[i..c].contains(&b'"') => {
            o.push(i, Role::Plain);
            o.push(c, Role::Func);
            o.push(c + 1, Role::Punct);
            yaml_value(&line[..body_end], c + 1, o);
        }
        _ => yaml_value(&line[..body_end], i, o),
    }
    if let Some(h) = hash {
        o.push(h, Role::Plain);
        o.push(line.len(), Role::Comment);
    }
    0
}

fn yaml_value(line: &[u8], from: usize, o: &mut Out) {
    let v = line[from..].trim_ascii_start();
    if v.starts_with(b"&") || v.starts_with(b"*") || v.starts_with(b"!") {
        let start = line.len() - v.len();
        let end = v
            .iter()
            .position(|c| c.is_ascii_whitespace())
            .map_or(line.len(), |p| start + p);
        o.push(start, Role::Plain);
        o.push(end, Role::Attr);
        value(line, end, o);
    } else {
        value(line, from, o);
    }
}
