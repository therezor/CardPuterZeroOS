//! The power dialog end to end: the real shell under `--sim`, keys from a
//! script. Under `--sim` the shell only logs what it would run, so nothing
//! on the host restarts.

use std::process::{Command, Stdio};

fn run(name: &str, keys: &str) -> String {
    let dir = std::env::temp_dir().join(format!("gh0st-power-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = dir.join("home/.config/cardputerzero");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.json"),
        "{\"gh0st\": {\"lock\": {\"password\": false}}}",
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
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Esc held opens the dialog; 7 held powers off, 5 held restarts.
#[test]
fn esc_held_then_a_held_chip_powers_off_or_restarts() {
    let open = "wait 500\ndown esc\nwait 900\nup esc\nwait 200\n";
    let off = run("off", &format!("{open}down 7\nwait 900\nup 7\nwait 300\n"));
    assert!(off.contains("gh0st shell: power off"), "{off}");
    let reboot = run(
        "reboot",
        &format!("{open}down 5\nwait 900\nup 5\nwait 300\n"),
    );
    assert!(reboot.contains("gh0st shell: power reboot"), "{reboot}");
}

#[test]
fn a_chip_let_go_early_says_to_hold_and_does_nothing() {
    let keys = "wait 500\ndown esc\nwait 900\nup esc\nwait 200\ndown 7\nwait 200\nup 7\nwait 300\n";
    let out = run("early", keys);
    assert!(out.contains("toast Hold to power off"), "{out}");
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
                down 7\nwait 900\nup 7\nwait 300\n";
    let out = run("search", keys);
    assert!(!out.contains("gh0st shell: power"), "{out}");
}

/// Power in home's menu opens the same dialog, and Esc closes it.
#[test]
fn the_menu_opens_the_dialog_and_esc_closes_it() {
    // 8 opens the menu; Power is its sixth row.
    let rows = "down down\nup down\n".repeat(5);
    let keys = format!(
        "wait 500\n8\nwait 100\n{rows}enter\nwait 200\nesc\nwait 200\ndown 7\nwait 900\nup 7\nwait 300\n"
    );
    let closed = run("menu", &keys);
    assert!(!closed.contains("gh0st shell: power"), "{closed}");
    let keys =
        format!("wait 500\n8\nwait 100\n{rows}enter\nwait 200\ndown 7\nwait 900\nup 7\nwait 300\n");
    let off = run("menu-off", &keys);
    assert!(off.contains("gh0st shell: power off"), "{off}");
}
