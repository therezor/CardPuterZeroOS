//! One binary, several programs. The name it runs under picks the program,
//! busybox style, so the shell and an open app share text pages in RAM.

use std::path::Path;

fn main() {
    let mut args = std::env::args();
    let argv0 = args.next().unwrap_or_default();
    let name = Path::new(&argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("cpz");
    let program = match name {
        "cpz" => args.next().unwrap_or_default(),
        other => other.to_string(),
    };
    match program.as_str() {
        "--version" | "version" => println!("cpz {}", env!("CARGO_PKG_VERSION")),
        "shell" | "cpz-shell" => std::process::exit(cpz_shell::run::main()),
        _ => {
            eprintln!("usage: cpz <shell|version>");
            std::process::exit(2);
        }
    }
}
