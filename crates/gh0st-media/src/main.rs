fn main() -> std::process::ExitCode {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().map(String::as_str) == Some("--fake-helper") {
        return gh0st_media::fake::main();
    }
    // A file to open, as the shell starts us for one. `--sim DIR` is the
    // SDK's.
    let mut file = None;
    while let Some(a) = args.next() {
        if a == "--sim" {
            args.next();
        } else if !a.starts_with("--") {
            file = Some(std::path::PathBuf::from(a));
        }
    }
    gh0st_app::run(gh0st_media::app::Media::new(file))
}
