//! HTML, XML and SVG. State 1 is inside a comment, 2 inside a tag, 3 and 4
//! inside a tag's quoted value (double, single) that runs on.

use super::{Out, Role};

pub(super) fn lex(state: u8, line: &[u8], o: &mut Out) -> u8 {
    let mut i = 0;
    let mut state = state;
    while i < line.len() {
        match state {
            1 => match find(line, i, b"-->") {
                Some(e) => {
                    o.push(e + 3, Role::Comment);
                    i = e + 3;
                    state = 0;
                }
                None => {
                    o.push(line.len(), Role::Comment);
                    return 1;
                }
            },
            3 | 4 => {
                let q = if state == 3 { b'"' } else { b'\'' };
                match line[i..].iter().position(|&c| c == q) {
                    Some(p) => {
                        o.push(i + p + 1, Role::String);
                        i += p + 1;
                        state = 2;
                    }
                    None => {
                        o.push(line.len(), Role::String);
                        return state;
                    }
                }
            }
            2 => {
                let c = line[i];
                if c == b'>' || (c == b'/' && line.get(i + 1) == Some(&b'>')) {
                    let e = if c == b'>' { i + 1 } else { i + 2 };
                    o.push(e, Role::Punct);
                    i = e;
                    state = 0;
                } else if c == b'"' || c == b'\'' {
                    state = if c == b'"' { 3 } else { 4 };
                    o.push(i + 1, Role::String);
                    i += 1;
                } else if c == b'=' {
                    o.push(i + 1, Role::Punct);
                    i += 1;
                } else if c.is_ascii_whitespace() {
                    o.push(i + 1, Role::Plain);
                    i += 1;
                } else {
                    let end = line[i..]
                        .iter()
                        .position(|&b| b.is_ascii_whitespace() || matches!(b, b'=' | b'>' | b'/'))
                        .map_or(line.len(), |p| i + p.max(1));
                    o.push(end, Role::Attr);
                    i = end;
                }
            }
            _ => {
                if line[i..].starts_with(b"<!--") {
                    o.push(i, Role::Plain);
                    state = 1;
                    o.push(i + 4, Role::Comment);
                    i += 4;
                } else if line[i] == b'<' {
                    // The tag's name: <p, </p, <?xml, <!DOCTYPE.
                    let mut j = i + 1;
                    while j < line.len() && matches!(line[j], b'/' | b'?' | b'!') {
                        j += 1;
                    }
                    o.push(j, Role::Punct);
                    let end = line[j..]
                        .iter()
                        .position(|&b| b.is_ascii_whitespace() || matches!(b, b'>' | b'/'))
                        .map_or(line.len(), |p| j + p);
                    o.push(end, Role::Keyword);
                    i = end;
                    state = 2;
                } else if line[i] == b'&' {
                    let end = line[i..]
                        .iter()
                        .take(12)
                        .position(|&b| b == b';')
                        .map_or(i + 1, |p| i + p + 1);
                    o.push(end, Role::Number);
                    i = end;
                } else {
                    let end = line[i..]
                        .iter()
                        .position(|&b| b == b'<' || b == b'&')
                        .map_or(line.len(), |p| i + p);
                    o.push(end.max(i + 1), Role::Plain);
                    i = end.max(i + 1);
                }
            }
        }
    }
    state
}

fn find(line: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    line[from..]
        .windows(pat.len())
        .position(|w| w == pat)
        .map(|p| from + p)
}
