//! The sandbox for real, where bwrap and user namespaces work: Linux CI
//! and the board. Elsewhere it says so and passes.

#[cfg(target_os = "linux")]
#[test]
fn an_app_in_its_sandbox_keeps_its_files_and_reaches_nothing_else() {
    use gh0st_core::sandbox::{self, BWRAP, Grants, Host};
    use std::path::Path;
    use std::process::Command;

    // A container masks parts of /proc, and then refuses a fresh one.
    let works = Command::new(BWRAP)
        .args(["--unshare-user", "--unshare-pid", "--dev-bind", "/", "/"])
        .args(["--proc", "/proc", "true"])
        .status()
        .is_ok_and(|s| s.success());
    if !works {
        eprintln!("skipped: no bwrap, no user namespaces, or no fresh /proc");
        return;
    }
    let root = std::env::temp_dir().join(format!("gh0st-sb-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (home, run) = (root.join("home"), root.join("run"));
    let notify = home.join(".local/state/gh0st/notify");
    for d in [".ssh", "Documents", ".local/share/gh0st-text"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(home.join(".ssh/id"), "secret").unwrap();
    std::fs::write(home.join(".bashrc"), "# mine\n").unwrap();
    std::fs::write(run.join("bus"), "").unwrap();
    sandbox::prepare(&home, &notify);
    // As the shell does before each start.
    std::fs::create_dir_all(sandbox::notice_dir(&notify, "zsynth")).unwrap();
    let host = Host::read(&home, &run);
    let probe = r#"
t() { if eval "$2" >/dev/null 2>&1; then echo "$1=yes"; else echo "$1=no"; fi; }
t ssh 'cat "$HOME/.ssh/id"'
t bashrc 'echo x >> "$HOME/.bashrc"'
t profile 'echo x > "$HOME/.bash_profile"'
t localbin 'touch "$HOME/.local/bin/ls"'
t text 'touch "$HOME/.local/share/gh0st-text/journal"'
t documents 'touch "$HOME/Documents/a"'
t newdir 'mkdir "$HOME/newdir"'
t notice 'touch "$HOME/.local/state/gh0st/notify/zsynth.hi"'
t pinned 'touch "$HOME/.local/state/gh0st/pinned"'
t runtime_bus 'test -e "$XDG_RUNTIME_DIR/bus"'
echo "pids=$(ls /proc | grep -c '^[0-9]')"
echo "links=$(tail -n +3 /proc/net/dev | wc -l)"
"#;
    let grants = Grants {
        network: false,
        ..Grants::default()
    };
    let inner = ["/bin/sh".into(), "-c".into(), probe.into()];
    let argv = sandbox::argv(&host, grants, "zsynth", &notify, &inner).unwrap();
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .env("HOME", &home)
        .env("XDG_RUNTIME_DIR", &run)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    let get = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .unwrap_or_else(|| panic!("no {k} in:\n{text}\n{err}"))
            .to_string()
    };
    for (k, want) in [
        ("ssh", "no"),
        ("bashrc", "no"),
        ("profile", "no"),
        ("localbin", "no"),
        ("text", "no"),
        ("pinned", "no"),
        ("documents", "yes"),
        ("newdir", "yes"),
        ("notice", "yes"),
        ("runtime_bus", "yes"),
    ] {
        assert_eq!(get(k), want, "{k}:\n{text}\n{err}");
    }
    assert!(get("pids").parse::<u32>().unwrap() < 10, "{text}");
    assert_eq!(get("links"), "1", "only loopback without the network");
    assert!(
        Path::new(&notify).join("zsynth/zsynth.hi").is_file(),
        "the notice lands in the app's own folder"
    );
    assert_eq!(
        std::fs::read_to_string(home.join(".bashrc")).unwrap(),
        "# mine\n"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
