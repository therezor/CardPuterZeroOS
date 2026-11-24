fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--fake-helper") {
        return gh0st_media::fake::main();
    }
    // The file the shell starts us with comes to Page::opened.
    gh0st_app::run(gh0st_media::app::Media::new(None))
}
