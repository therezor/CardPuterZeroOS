//! The spec's rules for object paths, interface, member, error and bus
//! names.

fn element(s: &str, digit_first: bool, dash: bool) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && (digit_first || !b[0].is_ascii_digit())
        && b.iter()
            .all(|&c| c.is_ascii_alphanumeric() || c == b'_' || (dash && c == b'-'))
}

pub fn path(s: &str) -> bool {
    if s == "/" {
        return true;
    }
    let Some(rest) = s.strip_prefix('/') else {
        return false;
    };
    rest.split('/')
        .all(|e| !e.is_empty() && e.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))
}

pub fn interface(s: &str) -> bool {
    s.len() <= 255 && s.split('.').count() >= 2 && s.split('.').all(|e| element(e, false, false))
}

pub fn error(s: &str) -> bool {
    interface(s)
}

pub fn member(s: &str) -> bool {
    s.len() <= 255 && element(s, false, false)
}

pub fn bus(s: &str) -> bool {
    if s.len() > 255 {
        return false;
    }
    match s.strip_prefix(':') {
        Some(rest) => {
            rest.split('.').count() >= 2 && rest.split('.').all(|e| element(e, true, true))
        }
        None => s.split('.').count() >= 2 && s.split('.').all(|e| element(e, false, true)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_spec() {
        assert!(path("/") && path("/org/freedesktop/secrets/aliases/default"));
        assert!(!path("") && !path("/a/") && !path("//a") && !path("a/b") && !path("/a-b"));
        assert!(
            interface("org.freedesktop.Secret.Service")
                && !interface("org")
                && !interface("org.1a")
        );
        assert!(member("GetSecrets") && !member("Get.Secrets") && !member("1x") && !member(""));
        assert!(bus(":1.42") && bus("org.freedesktop.secrets") && bus("org.gh0st-os.x"));
        assert!(!bus(":1") && !bus("org") && !bus("org.1x") && !bus(""));
    }
}
