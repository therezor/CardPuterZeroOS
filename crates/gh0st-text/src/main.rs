use gh0st_text::app::{Editor, Options};
use std::path::PathBuf;

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.first().map(String::as_str) == Some("--warm") {
        let from = raw.get(2).and_then(|f| f.parse().ok()).unwrap_or(0);
        let path = PathBuf::from(raw.get(1).cloned().unwrap_or_default());
        std::process::exit(gh0st_text::warm::main(&path, from));
    }
    // The first argument that is not a flag is the file. `--sim DIR` is the
    // SDK's.
    let mut args = std::env::args().skip(1);
    let mut path = None;
    while let Some(a) = args.next() {
        if a == "--sim" {
            args.next();
        } else if !a.starts_with("--") && path.is_none() {
            path = Some(PathBuf::from(a));
        }
    }
    gh0st_app::run(Editor::open(path.as_deref(), Options::default()));
}
