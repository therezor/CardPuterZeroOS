fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--fake-helper") {
        return gh0st_camera::fake::main();
    }
    gh0st_app::run(gh0st_camera::app::Camera::new())
}
