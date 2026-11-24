fn main() {
    // First-boot setup is the same program: it reuses Settings' pages.
    if std::env::args().any(|a| a == "--setup") {
        gh0st_settings::setup::run();
    }
    // The wallpaper's crop runs here, apart from the page's loop. The
    // portal names its own DST, kept only after the user's yes.
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, src, rest @ ..] = args.as_slice() {
        if flag == "--import-wallpaper" && rest.len() <= 1 {
            let dst = match rest.first() {
                Some(d) => d.into(),
                None => gh0st_settings::wallpaper::kept(),
            };
            match gh0st_settings::wallpaper::import(std::path::Path::new(src), &dst) {
                Ok(()) => std::process::exit(0),
                Err(e) => {
                    eprintln!("gh0st-settings: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
    gh0st_app::run(gh0st_settings::app::Settings::new());
}
