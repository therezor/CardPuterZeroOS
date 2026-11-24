//! One binary, several programs. The name it runs under picks the program,
//! busybox style, so the shell and an open app share text pages in RAM.

use std::path::Path;

mod audio;
mod auth;
mod helper;
mod open;
mod store;
mod update;

fn main() {
    let mut args = std::env::args();
    let argv0 = args.next().unwrap_or_default();
    let name = Path::new(&argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("gh0st");
    let program = match name {
        "gh0st" => args.next().unwrap_or_default(),
        other => other.to_string(),
    };
    match program.as_str() {
        "--version" | "version" => println!("gh0st {}", env!("CARGO_PKG_VERSION")),
        "shell" | "gh0st-shell" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(gh0st_shell::run::main(&rest))
        }
        "helper" | "gh0st-helper" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(helper::main(&rest))
        }
        "update" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(update::main(&rest))
        }
        "keyring" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(gh0st_keyringd::run::main(&rest))
        }
        "session" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(gh0st_session::run::main(&rest))
        }
        "audio" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(audio::main(&rest))
        }
        "store" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(store::main(&rest))
        }
        "open" | "xdg-open" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(open::main(&rest))
        }
        "screenshot" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(open::screenshot(&rest))
        }
        "pick-color" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(open::pick_color(&rest))
        }
        "wallpaper" => {
            let rest: Vec<String> = args.collect();
            std::process::exit(open::wallpaper(&rest))
        }
        _ => {
            eprintln!(
                "usage: gh0st <shell|helper|update|store|keyring|session|open|screenshot|pick-color|wallpaper|audio|version>"
            );
            std::process::exit(2);
        }
    }
}
