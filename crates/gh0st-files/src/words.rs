//! What Files says, in one place.

/// Help's About section.
pub const ABOUT: &str = "Tab flips between sides A and B. Copy and Move send \
what is marked to the other side. Server passwords stay in the keyring. \
Deleted files wait in the Trash.";

/// "1 item", "3 items".
pub fn items(n: usize) -> String {
    if n == 1 {
        "1 item".into()
    } else {
        format!("{n} items")
    }
}

/// "“notes.txt”" for one, "3 items" for more.
pub fn what(names: &[String]) -> String {
    match names {
        [one] => format!("\u{201c}{one}\u{201d}"),
        _ => items(names.len()),
    }
}

/// A fingerprint in groups of four, so a dialog can wrap it: the hash's
/// name goes, "SHA256:eV9BNqsF" gives "eV9B NqsF".
pub fn grouped(fp: &str) -> String {
    let body = fp.split_once(':').map_or(fp, |(_, b)| b);
    body.chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_names() {
        assert_eq!(items(1), "1 item");
        assert_eq!(what(&["a.txt".into()]), "\u{201c}a.txt\u{201d}");
        assert_eq!(what(&["a".into(), "b".into()]), "2 items");
        assert_eq!(grouped("SHA256:abcdefghi"), "abcd efgh i");
    }
}
