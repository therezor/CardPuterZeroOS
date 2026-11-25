//! One binary, several programs. The name it runs under picks the program,
//! busybox style, so the shell and an open app share text pages in RAM.

use std::ffi::OsString;
use std::path::Path;

mod audio;
mod auth;
mod helper;
mod open;
mod store;
mod update;

/// Arguments as text, for the programs that read text. A name that is not
/// UTF-8 is an error, not a panic.
fn text(args: impl Iterator<Item = OsString>) -> Vec<String> {
    args.map(|a| {
        a.into_string().unwrap_or_else(|a| {
            eprintln!("gh0st: not UTF-8: {}", a.to_string_lossy());
            std::process::exit(2)
        })
    })
    .collect()
}

fn main() {
    let mut args = std::env::args_os();
    let argv0 = args.next().unwrap_or_default();
    let name = Path::new(&argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("gh0st");
    let program = match name {
        "gh0st" => args
            .next()
            .and_then(|a| a.into_string().ok())
            .unwrap_or_default(),
        other => other.to_string(),
    };
    match program.as_str() {
        "--version" | "version" => println!("gh0st {}", env!("CARGO_PKG_VERSION")),
        "shell" | "gh0st-shell" => std::process::exit(gh0st_shell::run::main(&text(args))),
        "helper" | "gh0st-helper" => std::process::exit(helper::main(&text(args))),
        "update" => std::process::exit(update::main(&text(args))),
        "keyring" => std::process::exit(gh0st_keyringd::run::main(&text(args))),
        "session" => std::process::exit(gh0st_session::run::main(&text(args))),
        "audio" => std::process::exit(audio::main(&text(args))),
        "store" => std::process::exit(store::main(&text(args))),
        // Paths: any bytes.
        "open" | "xdg-open" => std::process::exit(open::main(&args.collect::<Vec<_>>())),
        "screenshot" => std::process::exit(open::screenshot(&args.collect::<Vec<_>>())),
        "pick-color" => std::process::exit(open::pick_color(&args.collect::<Vec<_>>())),
        "wallpaper" => std::process::exit(open::wallpaper(&args.collect::<Vec<_>>())),
        _ => {
            eprintln!(
                "usage: gh0st <shell|helper|update|store|keyring|session|open|screenshot|pick-color|wallpaper|audio|version>"
            );
            std::process::exit(2);
        }
    }
}
