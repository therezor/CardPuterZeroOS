//! Opening files end to end: the real shell under `--sim` with fake apps
//! that speak the link (a short Python program each, driven by a plan file),
//! so argv, hand-offs and the way back are the shell's own.

use gh0st_core::link::path_field;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A linked app. It logs its argv and every message, answers `pause`, and
/// follows `$SIM_DIR/<name>.plan`: `wait <ms>`, `on <verb>`, `send <fields>`
/// (tab-separated) and `exit`.
const APP: &str = r#"#!/usr/bin/env python3
import os, select, socket, sys, time
d = os.environ["SIM_DIR"]
me = os.path.basename(sys.argv[0])
def log(*a):
    with open(d + "/log", "a") as f:
        f.write(" ".join((me,) + a) + "\n")
log("argv", *sys.argv[1:])
s = socket.socket(fileno=int(os.environ["GH0ST_SHELL_FD"]))
def send(*f):
    s.send("\t".join(f).encode())
send("hello", "2")
got = []
def pump(t):
    r, _, _ = select.select([s], [], [], t)
    if not r:
        return
    m = s.recv(8192)
    if not m:
        sys.exit(0)
    f = m.decode("utf-8", "replace").split("\t")
    if f[0] == "pause":
        send("paused")
    log("got", *f)
    got.append(f[0])
for line in open(d + "/" + me + ".plan").read().splitlines():
    verb, _, rest = line.partition(" ")
    if verb == "wait":
        end = time.monotonic() + int(rest) / 1000
        while time.monotonic() < end:
            pump(end - time.monotonic())
    elif verb == "on":
        while rest not in got:
            pump(None)
        got.remove(rest)
    elif verb == "send":
        send(*rest.split("\t"))
    elif verb == "exit":
        sys.exit(0)
while True:
    pump(None)
"#;

struct Sim {
    dir: PathBuf,
}

impl Sim {
    fn new(name: &str) -> Sim {
        let dir = std::env::temp_dir().join(format!("gh0st-open-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = dir.join("home/.config/cardputerzero");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("config.json"),
            "{\"gh0st\": {\"lock\": {\"password\": false}}}",
        )
        .unwrap();
        for d in ["frames", "ours", "stock", "icons", "files"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        Sim { dir }
    }

    /// An app `name` with an `Exec` tail and extra `.desktop` lines.
    fn app(&self, name: &str, label: &str, args: &str, extra: &str, plan: &[String]) {
        let exe = self.dir.join(name);
        std::fs::write(&exe, APP).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            self.dir.join("ours").join(format!("{name}.desktop")),
            format!(
                "[Desktop Entry]\nType=Application\nName={label}\nExec={} {args}\n{extra}",
                exe.display()
            ),
        )
        .unwrap();
        std::fs::write(self.dir.join(format!("{name}.plan")), plan.join("\n")).unwrap();
    }

    fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.dir.join("files").join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// Runs the shell over `keys`; returns the apps' log and the shell's.
    fn run(&self, keys: &str) -> (String, String) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_gh0st"))
            .args(["shell", "--sim"])
            .arg(self.dir.join("frames"))
            .env("HOME", self.dir.join("home"))
            .env("XDG_CONFIG_HOME", self.dir.join("home/.config"))
            .env("GH0ST_APPS_OURS", self.dir.join("ours"))
            .env("GH0ST_APPS_STOCK", self.dir.join("stock"))
            .env("GH0ST_ICON_ROOT", self.dir.join("icons"))
            .env("GH0ST_LOG", "debug")
            .env("SIM_DIR", &self.dir)
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
        let log = std::fs::read_to_string(self.dir.join("log")).unwrap_or_default();
        (log, String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn send(fields: &[&str]) -> String {
    format!("send {}", fields.join("\t"))
}

fn open_file(p: &Path) -> String {
    send(&["open-file", &path_field(p)])
}

/// Lines of `log` in order, each found after the one before.
fn in_order(log: &str, want: &[&str]) {
    let mut at = 0;
    for w in want {
        match log[at..].find(w) {
            Some(i) => at += i + w.len(),
            None => panic!("missing, in order: {w}\n--- log\n{log}"),
        }
    }
}

#[test]
fn a_file_opens_in_its_app_and_each_app_returns_to_its_opener() {
    let s = Sim::new("chain");
    let notes = s.file("notes.txt", b"words\n");
    let more = s.file("more notes.txt", b"more\n");
    let pic = s.file("pic.png", b"\x89PNG\r\n\x1a\n0000");
    let blob = s.file("blob", b"\x00\x01\x02");
    // Alpha is on home. Beta opens text and Gamma pictures; neither shows.
    s.app(
        "a",
        "Alpha",
        "",
        "",
        &[
            "wait 400".into(),
            open_file(&notes),
            "on resume".into(),
            open_file(&blob),
            "wait 300".into(),
            "exit".into(),
        ],
    );
    s.app(
        "b",
        "Beta",
        "%f",
        "MimeType=text/plain;\nNoDisplay=true\n",
        &[
            "wait 400".into(),
            open_file(&pic),
            "on resume".into(),
            open_file(&more),
            "wait 300".into(),
            "exit".into(),
        ],
    );
    s.app(
        "c",
        "Gamma",
        "%F",
        "MimeType=image/*;\nNoDisplay=true\n",
        &["wait 400".into(), "exit".into()],
    );
    let (log, shell) = s.run("wait 600\nenter\nwait 5000\n");
    let n = notes.display().to_string();
    let p = pic.display().to_string();
    let m = more.display().to_string();
    in_order(
        &log,
        &[
            "a argv\n",
            &format!("b argv {n}\n"),
            &format!("c argv {p}\n"),
            "b got resume",
            &format!("b got open {m}"),
            "a got resume",
        ],
    );
    in_order(
        &shell,
        &[
            "started Alpha",
            "started Beta",
            "started Gamma",
            "resumed Beta",
            "resumed Alpha",
            "toast No app opens this file",
        ],
    );
}

#[test]
fn a_running_app_gets_the_file_and_an_unknown_app_says_so() {
    let s = Sim::new("handoff");
    let notes = s.file("notes.txt", b"words\n");
    let pic = s.file("pic.png", b"\x89PNG\r\n\x1a\n0000");
    // Alpha opens notes in Beta; Beta asks for Alpha by name with a picture;
    // Alpha, still running, gets it.
    s.app(
        "a",
        "Alpha",
        "%f",
        "MimeType=image/png;\n",
        &[
            "wait 400".into(),
            open_file(&notes),
            "on open".into(),
            send(&["open-with", "nobody", &path_field(&pic)]),
            "wait 300".into(),
            "exit".into(),
        ],
    );
    s.app(
        "b",
        "Beta",
        "%f",
        "MimeType=text/plain;\nNoDisplay=true\n",
        &[
            "wait 400".into(),
            send(&["open-with", "a", &path_field(&pic)]),
            "on resume".into(),
            "wait 200".into(),
            "exit".into(),
        ],
    );
    let (log, shell) = s.run("wait 600\nenter\nwait 4000\n");
    let p = pic.display().to_string();
    in_order(
        &log,
        &[
            "a argv\n",
            "b argv",
            "a got resume",
            &format!("a got open {p}"),
            "b got resume",
        ],
    );
    in_order(
        &shell,
        &[
            "started Beta",
            "resumed Alpha",
            "toast Can't find nobody",
            "resumed Beta",
        ],
    );
    assert_eq!(shell.matches("started Alpha").count(), 1, "{shell}");
}
