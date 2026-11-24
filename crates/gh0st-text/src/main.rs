use gh0st_text::app::{Editor, Options};
use std::path::PathBuf;

fn main() {
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
