//! Runs a command in the sandbox the shell would give app STEM, with the
//! user's grants: `sandboxed STEM -- CMD...`. For trying Store apps by
//! hand and for compat runs on the device.

fn main() {
    use gh0st_core::{apps, config, notify, sandbox};
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(stem), Some(sep)) = (args.first(), args.iter().position(|a| a == "--")) else {
        eprintln!("usage: sandboxed STEM -- CMD...");
        std::process::exit(2);
    };
    let inner: Vec<std::ffi::OsString> = args[sep + 1..].iter().map(Into::into).collect();
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let run = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let c = config::Config::load(&config::default_path());
    let key = |leaf| c.get_bool(&apps::app_key(&format!("{stem}.desktop"), leaf));
    let grants = sandbox::Grants {
        network: key("network").unwrap_or(true),
        camera: key("camera").unwrap_or(true),
        mic: key("mic").unwrap_or(true),
        input: key("input").unwrap_or(false),
    };
    let user = notify::user_dir();
    let _ = std::fs::create_dir_all(sandbox::notice_dir(&user, stem));
    let host = sandbox::Host::read(&home, &run);
    let Some(argv) = sandbox::argv(&host, grants, stem, &user, &inner) else {
        eprintln!("sandboxed: no home or runtime folder");
        std::process::exit(1);
    };
    let e = std::os::unix::process::CommandExt::exec(
        std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .env("PULSE_PROP", format!("gh0st.app={stem}")),
    );
    eprintln!("sandboxed: {e}");
    std::process::exit(1);
}
