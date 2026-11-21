fn main() {
    // First-boot setup is the same program: it reuses Settings' pages.
    if std::env::args().any(|a| a == "--setup") {
        gh0st_settings::setup::run();
    }
    gh0st_app::run(gh0st_settings::app::Settings::new());
}
