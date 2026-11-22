//! What setup keeps across a power cut: the step and the answers so far,
//! never the password. Root only.

use std::os::unix::fs::PermissionsExt;

pub const FILE: &str = "/var/lib/gh0st/setup.state";

pub struct Answers {
    /// The step open now, counting from 0.
    pub step: usize,
    /// The user took the tour rather than going straight to the settings.
    pub tour: bool,
    /// The account name typed so far.
    pub name: String,
    /// In memory only, until the account is made.
    pub password: String,
}

impl Default for Answers {
    fn default() -> Answers {
        Answers {
            step: 0,
            tour: true,
            name: String::new(),
            password: String::new(),
        }
    }
}

impl Drop for Answers {
    fn drop(&mut self) {
        std::mem::take(&mut self.password).into_bytes().fill(0);
    }
}

pub fn parse(text: &str) -> Answers {
    let mut a = Answers::default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("step", v)) => a.step = v.parse().unwrap_or(0),
            Some(("tour", v)) => a.tour = v != "0",
            Some(("name", v)) if gh0st_core::sysconf::valid_user_name(v) => a.name = v.into(),
            _ => {}
        }
    }
    a
}

/// Everything but the password.
pub fn format(a: &Answers) -> String {
    format!(
        "step={}\ntour={}\nname={}\n",
        a.step,
        u8::from(a.tour),
        a.name
    )
}

pub fn load() -> Answers {
    std::fs::read_to_string(super::path(FILE))
        .map(|t| parse(&t))
        .unwrap_or_default()
}

pub fn save(a: &Answers) {
    let path = super::path(FILE);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let r = gh0st_core::config::write_atomic(&path, format(a).as_bytes())
        .and_then(|()| std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)));
    if let Err(e) = r {
        eprintln!("setup: {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_never_reaches_the_file() {
        let a = Answers {
            step: 13,
            tour: false,
            name: "alex".into(),
            password: "hunter22".into(),
        };
        let text = format(&a);
        assert!(!text.contains("hunter22"));
        let b = parse(&text);
        assert_eq!((b.step, b.tour, b.name.as_str()), (13, false, "alex"));
        assert!(b.password.is_empty());
    }

    #[test]
    fn a_broken_file_starts_over() {
        let a = parse("step=x\nname=Not Valid\njunk\n");
        assert_eq!((a.step, a.tour, a.name.as_str()), (0, true, ""));
    }
}
