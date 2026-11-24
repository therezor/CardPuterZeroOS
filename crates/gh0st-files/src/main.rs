fn main() {
    // ssh runs this binary as its askpass program, with the socket to call.
    if let Ok(path) = std::env::var(gh0st_files::worker::askpass::ENV) {
        std::process::exit(gh0st_files::worker::askpass::client(&path));
    }
    if std::env::args().nth(1).as_deref() == Some("--worker") {
        std::process::exit(gh0st_files::worker::main());
    }
    gh0st_app::run(gh0st_files::app::Files::new());
}
