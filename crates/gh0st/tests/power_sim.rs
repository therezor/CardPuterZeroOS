//! The power dialog end to end: the real shell under `--sim`, keys from a
//! script. Under `--sim` the shell only logs what it would run, so nothing
//! on the host restarts.

use std::process::{Command, Stdio};

fn run(name: &str, keys: &str) -> String {
    run_frames(name, keys, "").0
}

/// The log and the last frame the shell drew, as PNG bytes.
/// `extra` adds keys to the scratch config.json.
fn run_frames(name: &str, keys: &str, extra: &str) -> (String, Vec<u8>) {
    let dir = std::env::temp_dir().join(format!("gh0st-power-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = dir.join("home/.config/cardputerzero");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.json"),
        format!("{{{extra}\"gh0st\": {{\"lock\": {{\"password\": false}}}}}}"),
    )
    .unwrap();
    for d in ["frames", "ours", "stock", "icons"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_gh0st"))
        .args(["shell", "--sim"])
        .arg(dir.join("frames"))
        .env("HOME", dir.join("home"))
        .env("GH0ST_APPS_OURS", dir.join("ours"))
        .env("GH0ST_APPS_STOCK", dir.join("stock"))
        .env("GH0ST_ICON_ROOT", dir.join("icons"))
        .env("GH0ST_LOG", "debug")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(keys.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let mut frames: Vec<_> = std::fs::read_dir(dir.join("frames"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    frames.sort();
    let last = frames
        .last()
        .map(|f| std::fs::read(f).unwrap())
        .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    (String::from_utf8_lossy(&out.stderr).into_owned(), last)
}

/// Esc held opens the dialog; 6 held restarts. 7, Off once, does nothing.
#[test]
fn esc_held_then_restart_held_restarts_and_there_is_no_off() {
    let open = "wait 500\ndown esc\nwait 900\nup esc\nwait 200\n";
    let reboot = run(
        "reboot",
        &format!("{open}down 6\nwait 900\nup 6\nwait 300\n"),
    );
    assert!(reboot.contains("gh0st shell: power reboot"), "{reboot}");
    let off = run("off", &format!("{open}down 7\nwait 900\nup 7\nwait 300\n"));
    assert!(!off.contains("gh0st shell: power"), "{off}");
}

#[test]
fn a_chip_let_go_early_says_to_hold_and_does_nothing() {
    let keys = "wait 500\ndown esc\nwait 900\nup esc\nwait 200\ndown 6\nwait 200\nup 6\nwait 300\n";
    let out = run("early", keys);
    assert!(out.contains("toast Hold to restart"), "{out}");
    assert!(!out.contains("gh0st shell: power"), "{out}");
}

#[test]
fn esc_tapped_on_home_says_to_hold_it() {
    let out = run("tap", "wait 500\nesc\nwait 300\n");
    assert!(out.contains("toast Hold Esc for power"), "{out}");
    assert!(!out.contains("gh0st shell: power"), "{out}");
}

/// Esc that leaves search is search's: held on, it opens nothing.
#[test]
fn esc_held_out_of_search_opens_nothing() {
    let keys = "wait 500\ns\nwait 200\ndown esc\nwait 900\nup esc\nwait 200\n\
                down 6\nwait 900\nup 6\nwait 300\n";
    let out = run("search", keys);
    assert!(!out.contains("gh0st shell: power"), "{out}");
}

/// Power in home's menu opens the same dialog, and Esc closes it.
#[test]
fn the_menu_opens_the_dialog_and_esc_closes_it() {
    // 8 opens the menu; Power is its sixth row.
    let rows = "down down\nup down\n".repeat(5);
    let keys = format!(
        "wait 500\n8\nwait 100\n{rows}enter\nwait 200\nesc\nwait 200\ndown 6\nwait 900\nup 6\nwait 300\n"
    );
    let closed = run("menu", &keys);
    assert!(!closed.contains("gh0st shell: power"), "{closed}");
    let keys =
        format!("wait 500\n8\nwait 100\n{rows}enter\nwait 200\ndown 6\nwait 900\nup 6\nwait 300\n");
    let reboot = run("menu-reboot", &keys);
    assert!(reboot.contains("gh0st shell: power reboot"), "{reboot}");
}

/// A restart draws the bye screen, which stays: keys after it do nothing.
#[test]
fn a_restart_says_so_and_takes_no_more_keys() {
    let keys = "wait 500\ndown esc\nwait 900\nup esc\nwait 200\n\
                down 6\nwait 900\nup 6\nwait 300\nesc\nwait 300\n8\nwait 300\n";
    // No wallpaper, so the frame is the bye screen pixel for pixel.
    let (out, last) = run_frames("bye", keys, "\"gh0st.wallpaper\": \"off\", ");
    assert!(out.contains("gh0st shell: power reboot"), "{out}");
    assert!(!out.contains("toast Hold Esc for power"), "{out}");
    let t = gh0st_ui::theme::Theme::new(gh0st_ui::theme::named(None));
    let mut c = gh0st_ui::Canvas::new(t.m.width, t.m.height);
    gh0st_shell::power::bye(&mut c, &t, true);
    let want = std::env::temp_dir().join(format!("gh0st-power-bye-{}.png", std::process::id()));
    gh0st_ui::snapshot::write_png(&c, &want, 2).unwrap();
    let want_png = std::fs::read(&want).unwrap();
    let _ = std::fs::remove_file(&want);
    assert!(last == want_png, "the last frame is not the bye screen");
}
