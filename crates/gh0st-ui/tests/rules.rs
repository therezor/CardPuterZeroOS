//! House rules that a scan of the source can hold: colour and faces come
//! from the theme, and no em dash anywhere (CLAUDE.md, Writing style).

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every file under `dir` with extension `ext`, skipping build output.
fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name();
        if name == "target" || name == "out" || name.to_string_lossy().starts_with('.') {
            continue;
        }
        if p.is_dir() {
            files(&p, ext, out);
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
}

/// The code of each `src/` file, up to its tests.
fn sources() -> Vec<(String, String)> {
    let mut v = Vec::new();
    files(&root().join("crates"), "rs", &mut v);
    v.into_iter()
        .filter(|p| p.components().any(|c| c.as_os_str() == "src"))
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            let code = text.split("#[cfg(test)]").next().unwrap_or("").to_string();
            let name = p.strip_prefix(root()).unwrap_or(&p).display().to_string();
            (name, code)
        })
        .collect()
}

fn is(name: &str, allowed: &[&str]) -> bool {
    allowed.iter().any(|a| name.ends_with(a))
}

#[test]
fn colours_come_from_the_theme() {
    // The theme names them, color.rs converts, the panel driver probes.
    let allowed = [
        "gh0st-ui/src/theme.rs",
        "gh0st-ui/src/color.rs",
        "gh0st-ui/src/fb.rs",
    ];
    let mut found = Vec::new();
    for (name, code) in sources() {
        if is(&name, &allowed) {
            continue;
        }
        for (i, line) in code.lines().enumerate() {
            let literal = line.match_indices("rgb(").any(|(at, _)| {
                line[at + 4..]
                    .trim_start()
                    .starts_with(|c: char| c.is_ascii_digit())
            });
            if literal && !line.trim_start().starts_with("//") {
                found.push(format!("{name}:{}: {}", i + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "name the colour in theme.rs:\n{}",
        found.join("\n")
    );
}

#[test]
fn faces_come_from_the_theme() {
    let allowed = [
        "gh0st-ui/src/theme.rs",
        "gh0st-ui/src/font.rs",
        "gh0st-ui/src/lint.rs",
    ];
    let faces = ["font::SANS_", "font::MONO_", "font::TERMINUS_"];
    let mut found = Vec::new();
    for (name, code) in sources() {
        if is(&name, &allowed) {
            continue;
        }
        for (i, line) in code.lines().enumerate() {
            if faces.iter().any(|f| line.contains(f)) {
                found.push(format!("{name}:{}: {}", i + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "take the face from Theme::f:\n{}",
        found.join("\n")
    );
}

#[test]
fn no_em_dash_anywhere() {
    let mut paths = Vec::new();
    files(&root().join("crates"), "rs", &mut paths);
    files(&root().join("docs"), "md", &mut paths);
    paths.push(root().join("README.md"));
    let mut found = Vec::new();
    for p in paths {
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        for (i, line) in text.lines().enumerate() {
            if line.contains('\u{2014}') {
                found.push(format!("{}:{}", p.display(), i + 1));
            }
        }
    }
    assert!(
        found.is_empty(),
        "use a full stop, a comma, a colon or brackets:\n{}",
        found.join("\n")
    );
}
