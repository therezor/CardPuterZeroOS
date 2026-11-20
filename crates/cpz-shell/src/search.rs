//! Type-to-search ranking, from pt35's launcher.

/// How well `needle` matches `hay`, lower is better, `None` for no match: the
/// start of it, the start of a word, anywhere, then its letters in order.
/// Both sides should already be lower case.
pub fn score(hay: &str, needle: &str) -> Option<u8> {
    if needle.is_empty() || hay.starts_with(needle) {
        return Some(0);
    }
    let word_start = hay
        .match_indices(needle)
        .any(|(at, _)| hay[..at].ends_with([' ', '-', '_', '.', ':', '/']));
    if word_start {
        return Some(2);
    }
    if hay.contains(needle) {
        return Some(4);
    }
    let mut rest = hay.chars();
    needle.chars().all(|c| rest.any(|h| h == c)).then_some(6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_beats_a_word_start_beats_anywhere_beats_scattered() {
        assert_eq!(score("terminal", "ter"), Some(0));
        assert_eq!(score("keyboard guide", "gui"), Some(2));
        assert_eq!(score("store", "re"), Some(4));
        assert_eq!(score("settings", "stg"), Some(6));
        assert_eq!(score("camera", "x"), None);
    }
}
