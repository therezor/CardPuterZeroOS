//! Sound end to end: the real shell under `--sim`, keys from a script.
//! A paused app's streams: the shell tells `gh0st audio` to park the group
//! when it stops an app, and to unpark it when the app comes back or is
//! gone. System sounds: the shell writes them to `pacat`. Stubs stand in
//! for both and log what they get.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

const AUDIO: &str = "#!/bin/sh\necho \"$*\" >> \"$SIM_DIR/log\"\n";
const PACAT: &str = "#!/bin/sh\nexec cat >> \"$SIM_DIR/pcm\"\n";
const SLEEP: &str = "#!/bin/sh\nexec sleep 30\n";
/// An app that turns system sounds off, as Settings does, then waits.
const SILENCER: &str = "#!/bin/sh\n\
    printf '{\"gh0st\": {\"lock\": {\"password\": false}, \"sounds\": false}}' \
    > \"$HOME/.config/cardputerzero/config.json\"\nexec sleep 30\n";

fn executable(p: &Path, body: &str) {
    std::fs::write(p, body).unwrap();
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

struct Run {
    /// What `gh0st audio` was asked, a line each.
    log: String,
    /// Bytes `pacat` was given.
    pcm: usize,
    /// All of them zero: silence.
    silent: bool,
    stderr: String,
}

/// Runs the shell with `apps` on home, each running `program`, and the
/// lock off, plus `config`'s keys under `gh0st` and `top`'s beside it.
fn run(name: &str, top: &str, config: &str, apps: &[&str], program: &str, keys: &str) -> Run {
    let dir = std::env::temp_dir().join(format!("gh0st-sound-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home/.config/cardputerzero");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.json"),
        format!("{{{top}\"gh0st\": {{\"lock\": {{\"password\": false}}{config}}}}}"),
    )
    .unwrap();
    for d in ["frames", "ours", "stock", "icons"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    executable(&dir.join("audio"), AUDIO);
    executable(&dir.join("pacat"), PACAT);
    // One program each: home shows one tile per program.
    let body = program;
    for app in apps {
        let program = dir.join(format!("run-{app}"));
        executable(&program, body);
        std::fs::write(
            dir.join(format!("ours/{app}.desktop")),
            format!(
                "[Desktop Entry]\nType=Application\nName={app}\nExec={}\n",
                program.display()
            ),
        )
        .unwrap();
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_gh0st"))
        .args(["shell", "--sim"])
        .arg(dir.join("frames"))
        .env("HOME", dir.join("home"))
        .env("GH0ST_APPS_OURS", dir.join("ours"))
        .env("GH0ST_APPS_STOCK", dir.join("stock"))
        .env("GH0ST_ICON_ROOT", dir.join("icons"))
        .env("GH0ST_AUDIO_EXE", dir.join("audio"))
        .env("GH0ST_PACAT", dir.join("pacat"))
        .env("GH0ST_LOG", "debug")
        .env("SIM_DIR", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(keys.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    // The stubs run detached; give the last one a moment to write.
    std::thread::sleep(std::time::Duration::from_millis(200));
    let pcm = std::fs::read(dir.join("pcm")).unwrap_or_default();
    let run = Run {
        log: std::fs::read_to_string(dir.join("log")).unwrap_or_default(),
        pcm: pcm.len(),
        silent: pcm.iter().all(|&b| b == 0),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    };
    let _ = std::fs::remove_dir_all(&dir);
    run
}

#[test]
fn a_paused_app_is_parked_and_unparked() {
    // Open, pause, resume from the switcher, pause again, close it there.
    let keys = "wait 400\nenter\nwait 600\nhome\nwait 400\nenter\nwait 600\n\
                home\nwait 400\nbackspace\nwait 1500\n";
    let r = run("park", "", "", &["Player"], SLEEP, keys);
    let calls: Vec<&str> = r.log.lines().collect();
    assert_eq!(
        calls.first(),
        Some(&"audio clear"),
        "{}\n{}",
        r.log,
        r.stderr
    );
    let parked = calls
        .iter()
        .find_map(|l| l.strip_prefix("audio park "))
        .unwrap_or_else(|| panic!("never parked:\n{}\n{}", r.log, r.stderr));
    // The group, then the app: a sandboxed app's streams carry its name.
    let (pgid, app) = parked.split_once(' ').unwrap_or((parked, ""));
    assert!(!app.is_empty(), "{}", r.log);
    let park = format!("audio park {pgid} {app}");
    let unpark = format!("audio unpark {pgid}");
    assert_eq!(
        calls[1..],
        [&park, &unpark, &park, &unpark].map(String::as_str),
        "{}\n{}",
        r.log,
        r.stderr
    );
}

#[test]
fn moving_on_home_sounds_unless_sounds_are_off() {
    let keys = "wait 400\nright\nwait 300\nleft\nwait 2600\n";
    let on = run("on", "", "", &["Alpha", "Beta"], SLEEP, keys);
    assert!(on.pcm > 0, "no sound: {}", on.stderr);
    let off = run(
        "off",
        "",
        ", \"sounds\": false",
        &["Alpha", "Beta"],
        SLEEP,
        keys,
    );
    assert_eq!(off.pcm, 0, "{}", off.stderr);
}

#[test]
fn sounds_turned_off_in_an_app_stay_off_once_it_leaves_the_front() {
    // Open it (a sound), pause it, move in the switcher (a tick, or not).
    let keys = "wait 400\nenter\nwait 800\nhome\nwait 400\nleft\nwait 2600\n";
    let kept = run("kept", "", "", &["Alpha"], SLEEP, keys);
    let off = run("silenced", "", "", &["Alpha"], SILENCER, keys);
    assert!(off.pcm > 0, "the open sounds first: {}", off.stderr);
    assert!(
        kept.pcm > off.pcm,
        "{} vs {}: {}",
        kept.pcm,
        off.pcm,
        off.stderr
    );
}

/// Dark a second after the start.
const DARK: &str = "\"dark_time\": 1, ";

#[test]
fn a_wake_opens_the_stream_before_the_first_key() {
    // Dark, then a key that wakes it and makes no sound of its own.
    let keys = "wait 2000\nright\nwait 2600\n";
    let r = run("wake", DARK, "", &["Alpha", "Beta"], SLEEP, keys);
    assert!(r.pcm > 0, "no stream at the wake: {}", r.stderr);
    assert!(r.silent, "{}", r.stderr);
    let off = run(
        "wake-off",
        DARK,
        ", \"sounds\": false",
        &["Alpha"],
        SLEEP,
        keys,
    );
    assert_eq!(off.pcm, 0, "{}", off.stderr);
}
