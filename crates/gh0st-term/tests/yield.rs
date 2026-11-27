//! A program that opens the panel gets the screen: the terminal draws
//! nothing and every key is the program's, Esc too, until it closes it.
//! The panel here is a plain file, through `LV_LINUX_FBDEV_DEVICE`.
#![cfg(target_os = "linux")]

use gh0st_app::testing::Driver;
use gh0st_term::app::{Options, Term};

#[test]
fn a_program_with_the_panel_has_the_screen_and_the_keys() {
    let dir = std::env::temp_dir().join(format!("gh0st-term-yield-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (panel, out) = (dir.join("panel"), dir.join("out"));
    std::fs::write(&panel, b"").unwrap();
    // SAFETY: the only test in this binary, before any thread reads the env.
    unsafe { std::env::set_var("LV_LINUX_FBDEV_DEVICE", &panel) };
    let script = format!(
        "exec 3<>'{}'; stty raw -echo; head -c1 > '{}'; exec 3>&-; exec cat",
        panel.display(),
        out.display()
    );
    let opts = Options {
        command: Some(vec!["/bin/sh".into(), "-c".into(), script]),
        sysplause: false,
    };
    let mut d = Driver::new(Term::new(opts));
    d.pump(500);
    let before = d.frames();
    d.wait(3000);
    assert_eq!(d.frames(), before, "drew over the program");
    d.keys("esc").pump(500);
    assert!(!d.menu_open(), "Esc is the program's");
    assert_eq!(std::fs::read(&out).ok().as_deref(), Some(&b"\x1b"[..]));
    d.wait(100);
    assert!(
        d.frames() > before,
        "panel closed: the terminal draws again"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
