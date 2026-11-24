//! Diffs and patches: each line says what it is by its first bytes.

use super::{Out, Role};

pub(super) fn lex(line: &[u8], o: &mut Out) -> u8 {
    let role = if line.starts_with(b"+++") || line.starts_with(b"---") {
        Role::Heading
    } else if line.starts_with(b"@@") {
        Role::Type
    } else if line.starts_with(b"+") || line.starts_with(b">") {
        Role::Added
    } else if line.starts_with(b"-") || line.starts_with(b"<") {
        Role::Removed
    } else if line.starts_with(b"diff ") || line.starts_with(b"index ") {
        Role::Comment
    } else {
        Role::Plain
    };
    o.push(line.len(), role);
    0
}
