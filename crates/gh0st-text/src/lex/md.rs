//! Markdown. State 1 is inside a fenced code block.

use super::{Out, Role};

pub(super) fn lex(state: u8, line: &[u8], o: &mut Out) -> u8 {
    let trimmed = line.trim_ascii_start();
    let fence = trimmed.starts_with(b"```") || trimmed.starts_with(b"~~~");
    if state == 1 {
        o.push(line.len(), if fence { Role::Punct } else { Role::String });
        return if fence { 0 } else { 1 };
    }
    if fence {
        let lang_at = line.len() - trimmed.len() + 3;
        o.push(lang_at, Role::Punct);
        o.push(line.len(), Role::Type);
        return 1;
    }
    if trimmed.starts_with(b"#") {
        o.push(line.len(), Role::Heading);
        return 0;
    }
    if trimmed.starts_with(b">") {
        o.push(line.len(), Role::Comment);
        return 0;
    }
    let lead = line.len() - trimmed.len();
    let mut i = 0;
    // List markers and rules.
    if trimmed.starts_with(b"- ") || trimmed.starts_with(b"* ") || trimmed.starts_with(b"+ ") {
        o.push(lead + 1, Role::Punct);
        i = lead + 1;
    } else if trimmed
        .iter()
        .all(|&c| matches!(c, b'-' | b'*' | b'_' | b' '))
        && trimmed.len() >= 3
    {
        o.push(line.len(), Role::Punct);
        return 0;
    } else {
        let digits = trimmed.iter().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && matches!(trimmed.get(digits), Some(b'.' | b')')) {
            o.push(lead + digits + 1, Role::Punct);
            i = lead + digits + 1;
        }
    }
    inline(line, i, o);
    0
}

/// Code spans, links and emphasis.
fn inline(line: &[u8], from: usize, o: &mut Out) {
    let mut i = from;
    while i < line.len() {
        match line[i] {
            b'`' => {
                let end = line[i + 1..]
                    .iter()
                    .position(|&c| c == b'`')
                    .map_or(line.len(), |p| i + p + 2);
                o.push(i, Role::Plain);
                o.push(end, Role::String);
                i = end;
            }
            b'[' => {
                // [text](url): the text as a link, the url as a string.
                let close = line[i..].iter().position(|&c| c == b']').map(|p| i + p);
                match close {
                    Some(c) if line.get(c + 1) == Some(&b'(') => {
                        let end = line[c..]
                            .iter()
                            .position(|&b| b == b')')
                            .map_or(line.len(), |p| c + p + 1);
                        o.push(i, Role::Plain);
                        o.push(c + 1, Role::Func);
                        o.push(end, Role::String);
                        i = end;
                    }
                    _ => i += 1,
                }
            }
            b'*' | b'_' if line.get(i + 1) == Some(&line[i]) => {
                let m = line[i];
                let end = line[i + 2..]
                    .windows(2)
                    .position(|w| w[0] == m && w[1] == m)
                    .map_or(line.len(), |p| i + 2 + p + 2);
                o.push(i, Role::Plain);
                o.push(end, Role::Keyword);
                i = end;
            }
            _ => i += 1,
        }
    }
    o.push(line.len(), Role::Plain);
}
