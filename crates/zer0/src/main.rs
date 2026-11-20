//! One binary, several programs. The name it runs under picks the program,
//! busybox style, so the shell and an open app share text pages in RAM.

use std::path::Path;

fn main() {
    let mut args = std::env::args();
    let argv0 = args.next().unwrap_or_default();
    let name = Path::new(&argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("zer0");
    let program = match name {
        "zer0" => args.next().unwrap_or_default(),
        other => other.to_string(),
    };
    match program.as_str() {
        "--version" | "version" => println!("zer0 {}", env!("CARGO_PKG_VERSION")),
        "shell" | "zer0-shell" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(zer0_shell::run::main(&rest))
        }
        _ => {
            eprintln!("usage: zer0 <shell|version>");
            std::process::exit(2);
        }
    }
}
