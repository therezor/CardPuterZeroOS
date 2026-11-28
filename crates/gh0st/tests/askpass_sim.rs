//! sudo's question, end to end: the real shell under `--sim`, a root app
//! that asks, a stand-in for sudo that runs the real helper, and a checker
//! that validates. Keys go in one step at a time, since the prompt prints
//! nothing and its arm window starts at its first frame. Nothing on the
//! host changes.

use std::io::{Read, Write};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

const READY: Duration = Duration::from_secs(8);
const SETTLE: Duration = Duration::from_millis(700);

/// Holds at the ready marker until released, then asks sudo's question.
const RUN_ROOT: &str = r#"#!/bin/sh
: > "$T9/ready"
while [ ! -e "$T9/go" ]; do /bin/sleep 0.02; done
exec sudo true
"#;

/// Exits 2 at once, after writing down what it was given.
const RUN_OFF: &str = r#"#!/bin/sh
for v in DISPLAY SUDO_ASKPASS GH0ST_ASKPASS; do
    if [ -n "${!v+x}" ]; then echo "$v=set"; else echo "$v=unset"; fi
done > "$T9/env"
exit 2
"#;

/// Alive and in front, asking nothing, until released.
const RUN_INERT: &str = r#"#!/bin/sh
: > "$T9/ready"
while [ ! -e "$T9/go" ]; do /bin/sleep 0.02; done
exit 0
"#;

/// sudo's stand-in: writes down what the app was given, asks through the
/// helper, keeps its answer and status, and succeeds on a non-empty one.
const SUDO: &str = r#"#!/bin/sh
{
    echo "DISPLAY=${DISPLAY-unset}"
    echo "SUDO_ASKPASS=${SUDO_ASKPASS-unset}"
    echo "GH0ST_ASKPASS=${GH0ST_ASKPASS-unset}"
} > "$T9/env"
echo call >> "$T9/calls"
out=$("$SUDO_ASKPASS" "[sudo] password for pi: " 2>"$T9/helper.err")
rc=$?
printf '%s' "$out" > "$T9/reply"
echo "$rc" > "$T9/helper.rc"
[ -n "$out" ] && exit 0
exit 1
"#;

/// `unix_chkpwd`'s stand-in: one password is right, anything else is a
/// wrong one. The log never holds what was typed.
const CHKPWD: &str = r#"#!/bin/sh
/bin/cat > "$T9/chkpwd.in"
echo "chkpwd $*" >> "$T9/log"
printf 'hunter22\0' > "$T9/expect"
/usr/bin/cmp -s "$T9/chkpwd.in" "$T9/expect" && exit 0
exit 7
"#;

const PASSWD: &str = r#"#!/bin/sh
echo "me P 2026-11-23 0 99999 7 -1"
"#;

const CATALOG: &str = "gh0st-store 1\ndate 4102444800\nexpires 4102444800\n\
app\trootprobe\t1.0\t1024\t\
0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\t\
tools\troot\tRoot Probe\tAsks for root\tus\tMIT\t\thttps://x\thttps://x\trootprobe\n";

fn desktop(name: &str, exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={}\n",
        exe.display()
    )
}

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

/// A private runtime dir of this shell's own, short enough for a socket
/// path, so sims in parallel never share the askpass socket or the
/// instance lock.
fn runtime_dir(case: &str) -> PathBuf {
    let rt = PathBuf::from(format!("/tmp/gh0st-{}-{case}", std::process::id()));
    let _ = std::fs::remove_dir_all(&rt);
    std::fs::create_dir_all(&rt).unwrap();
    rt
}

struct Fixture {
    dir: PathBuf,
    home: PathBuf,
    frames: PathBuf,
    /// `$XDG_RUNTIME_DIR` of this shell.
    rt: PathBuf,
    /// Where the shell's askpass socket is, for a stranger to knock on.
    socket: PathBuf,
    helper: PathBuf,
    passwd: PathBuf,
    chkpwd: PathBuf,
}

impl Fixture {
    /// `stock`: desktop file stems with their `Name`, and their program.
    fn new(case: &str, stock: &[(&str, &str)]) -> Fixture {
        let dir = std::env::temp_dir().join(format!("gh0st-t9-{case}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = dir.join("home");
        let ours = dir.join("ours");
        let apps = dir.join("stock");
        let icons = dir.join("icons");
        let bin = dir.join("bin");
        let store = dir.join("store");
        let frames = dir.join("frames");
        let rt = runtime_dir(case);
        for d in [&home, &ours, &apps, &icons, &bin, &store, &frames] {
            std::fs::create_dir_all(d).unwrap();
        }
        let settings = home.join(".config/gh0st");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(
            settings.join("settings.json"),
            "{\"gh0st.lock.password\":false,\"gh0st.app.offprobe_desktop.isolate\":false}",
        )
        .unwrap();
        for (stem, name) in stock {
            let exe = script(&bin, &format!("run-{stem}"), run_for(stem));
            std::fs::write(apps.join(format!("{stem}.desktop")), desktop(name, &exe)).unwrap();
        }
        script(&bin, "sudo", SUDO);
        std::fs::write(store.join("catalog"), CATALOG).unwrap();
        let helper = dir.join("gh0st-askpass");
        symlink(env!("CARGO_BIN_EXE_gh0st"), &helper).unwrap();
        let passwd = script(&dir, "passwd", PASSWD);
        let chkpwd = script(&dir, "chkpwd", CHKPWD);
        // The shell keeps its socket in a folder of its own uid's.
        let socket = rt
            .join(format!("gh0st-askpass-{}", unsafe { libc::getuid() }))
            .join("askpass");
        Fixture {
            dir,
            home,
            frames,
            rt,
            socket,
            helper,
            passwd,
            chkpwd,
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path(rel)).unwrap_or_default()
    }

    /// The newest frame's number, or none.
    fn last_frame(&self) -> Option<u32> {
        std::fs::read_dir(&self.frames)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter_map(|n| n.strip_prefix("frame-")?.strip_suffix(".png")?.parse().ok())
            .max()
    }

    fn wait_for(&self, rel: &str) -> bool {
        let until = Instant::now() + READY;
        while Instant::now() < until {
            if self.exists(rel) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Waits for a frame newer than `after`.
    fn wait_frame(&self, after: Option<u32>) -> Option<u32> {
        let until = Instant::now() + READY;
        loop {
            let n = self.last_frame();
            if n > after && n.is_some() {
                return n;
            }
            if Instant::now() >= until {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits a while and tells whether a frame came in that time.
    fn frame_came_in(&self, after: Option<u32>) -> bool {
        std::thread::sleep(SETTLE);
        newer(self.last_frame(), after)
    }

    /// The shell, stdin held open, without what a desktop would give it.
    fn spawn(&self) -> Child {
        let mut path = self.dir.join("bin").into_os_string();
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());
        Command::new(env!("CARGO_BIN_EXE_gh0st"))
            .args(["shell", "--sim"])
            .arg(&self.frames)
            .env("HOME", &self.home)
            .env("GH0ST_APPS_OURS", self.path("ours"))
            .env("GH0ST_APPS_STOCK", self.path("stock"))
            .env("GH0ST_ICON_ROOT", self.path("icons"))
            .env("GH0ST_STORE_STATE", self.path("store"))
            .env("GH0ST_ASKPASS_EXE", &self.helper)
            .env("GH0ST_CHKPWD", &self.chkpwd)
            .env("GH0ST_PASSWD", &self.passwd)
            .env("T9", &self.dir)
            .env("PATH", path)
            .env("XDG_RUNTIME_DIR", &self.rt)
            .env_remove("DISPLAY")
            .env_remove("SUDO_ASKPASS")
            .env_remove("GH0ST_ASKPASS")
            .env_remove("GH0ST_LOG")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn keys(&self, stdin: &mut ChildStdin, lines: &[&str]) {
        for l in lines {
            writeln!(stdin, "{l}").unwrap();
        }
        stdin.flush().unwrap();
    }

    /// Opens the first tile and waits for its ready marker.
    fn launch(&self, stdin: &mut ChildStdin) -> bool {
        self.keys(stdin, &["wait 600", "enter"]);
        self.wait_for("ready")
    }

    /// Releases the app, then waits for the prompt's first frame.
    fn ask(&self) -> Option<u32> {
        let before = self.last_frame();
        std::fs::write(self.path("go"), "").unwrap();
        self.wait_frame(before)
    }

    /// Lets the script drain, closes stdin and collects stderr. `stdin`
    /// is the pipe when the case held it.
    fn finish(&self, mut child: Child, stdin: Option<ChildStdin>, tail: &str) -> String {
        let mut stdin = stdin.or_else(|| child.stdin.take()).unwrap();
        self.keys(&mut stdin, &[tail]);
        drop(stdin);
        let out = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    /// Connects as a stranger: EOF with nothing back, or what came.
    fn stranger(&self) -> Result<usize, String> {
        let mut s = UnixStream::connect(&self.socket).map_err(|e| e.to_string())?;
        s.set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        let mut buf = [0u8; 16];
        match s.read(&mut buf) {
            Ok(0) | Err(_) => Ok(0),
            Ok(n) => Ok(n),
        }
    }
}

/// Whether `a` is a frame newer than `b`.
fn newer(a: Option<u32>, b: Option<u32>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a > b,
        (Some(_), None) => true,
        _ => false,
    }
}

fn run_for(stem: &str) -> &'static str {
    match stem {
        "rootprobe" => RUN_ROOT,
        "offprobe" => RUN_OFF,
        "inert" => RUN_INERT,
        _ => panic!("no program for {stem}"),
    }
}

fn notice(f: &Fixture, id: &str) -> Option<String> {
    std::fs::read_to_string(f.home.join(".local/state/gh0st/notify").join(id)).ok()
}

/// Lets the shell reap the app and leave the prompt before stdin closes.
const DRAIN: &str = "wait 800";

#[test]
fn a_root_app_gets_the_password_through_the_shell() {
    let f = Fixture::new("password", &[("rootprobe", "Root Probe")]);
    let mut child = f.spawn();
    let mut stdin = child.stdin.take().unwrap();
    assert!(f.launch(&mut stdin), "the probe never started");
    let prompt = f.ask();
    assert!(prompt.is_some(), "no prompt frame after the ask");
    f.keys(&mut stdin, &["wait 600", "type hunter22", "enter"]);
    assert!(f.wait_for("reply"), "sudo never got an answer");
    let stderr = f.finish(child, Some(stdin), DRAIN);

    assert_eq!(f.read("reply"), "hunter22");
    assert_eq!(f.read("helper.rc").trim(), "0");
    assert_eq!(f.read("calls").lines().count(), 1, "one question");
    assert_eq!(
        f.read("log")
            .lines()
            .filter(|l| l.starts_with("chkpwd"))
            .count(),
        1,
        "the checker ran once"
    );
    let env = f.read("env");
    let socket = f.socket.display().to_string();
    let helper = f.helper.display().to_string();
    for line in [
        "DISPLAY=:0".to_string(),
        format!("SUDO_ASKPASS={helper}"),
        format!("GH0ST_ASKPASS={socket}"),
    ] {
        assert!(
            env.lines().any(|l| l == line),
            "{line:?} missing from:\n{env}"
        );
    }
    assert!(
        stderr.contains("Root Probe exited with status 0 after"),
        "the exit line names the status: {stderr}"
    );
    assert_eq!(
        notice(&f, "settings.exit-rootprobe"),
        None,
        "a zero exit is no notice"
    );
}

#[test]
fn esc_refuses_the_question_and_the_notice_says_root() {
    let f = Fixture::new("esc", &[("rootprobe", "Root Probe")]);
    let mut child = f.spawn();
    let mut stdin = child.stdin.take().unwrap();
    assert!(f.launch(&mut stdin), "the probe never started");
    let prompt = f.ask();
    assert!(prompt.is_some(), "no prompt frame after the ask");
    f.keys(&mut stdin, &["esc"]);
    let stderr = f.finish(child, Some(stdin), DRAIN);

    assert_eq!(f.read("reply"), "", "nothing went to sudo");
    assert!(
        f.read("helper.err").contains("refused"),
        "{}",
        f.read("helper.err")
    );
    assert_eq!(f.read("calls").lines().count(), 1, "one question");
    assert_eq!(
        f.read("log").lines().count(),
        0,
        "nothing was typed, nothing checked"
    );
    assert!(
        stderr.contains("Root Probe exited with status 1 after"),
        "the exit line names the status: {stderr}"
    );
    let n = notice(&f, "settings.exit-rootprobe").expect("the notice");
    assert!(n.contains("title=Root Probe closed as it opened"), "{n}");
    assert!(
        n.contains("body=It runs as root. A password prompt you dismissed, or its own error."),
        "{n}"
    );
}

#[test]
fn an_app_with_isolation_off_gets_the_other_notice_and_no_env() {
    let f = Fixture::new("off", &[("offprobe", "Off Probe")]);
    // The fixture is what Settings produces for Isolation off.
    let dirs = gh0st_core::apps::Dirs {
        ours: f.path("ours"),
        stock: f.path("stock"),
        icon_root: f.path("icons"),
    };
    let config =
        gh0st_core::config::Config::load(&f.home.join(".config/cardputerzero/config.json"));
    let app = gh0st_core::apps::scan(&dirs, &config, &[])
        .into_iter()
        .find(|a| a.file == "offprobe.desktop")
        .expect("the probe");
    assert!(!gh0st_core::apps::isolation(&config, &app, false).sandbox);

    let child = f.spawn();
    let stderr = f.finish(child, None, "wait 600\nenter\nwait 1200");

    let env = f.read("env");
    for line in ["DISPLAY=unset", "SUDO_ASKPASS=unset", "GH0ST_ASKPASS=unset"] {
        assert!(
            env.lines().any(|l| l == line),
            "{line:?} missing from:\n{env}"
        );
    }
    assert!(!f.exists("calls"), "it never asked");
    assert!(
        stderr.contains("Off Probe exited with status 2 after"),
        "the exit line names the status: {stderr}"
    );
    let n = notice(&f, "settings.exit-offprobe").expect("the notice");
    assert!(n.contains("title=Off Probe closed as it opened"), "{n}");
    assert!(
        n.contains("body=Isolation is off for it, so its sandbox isn't why. Its own error."),
        "{n}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_stranger_with_an_app_in_front_is_closed_without_a_reply() {
    let f = Fixture::new("stranger-front", &[("inert", "Inert")]);
    let mut child = f.spawn();
    let mut stdin = child.stdin.take().unwrap();
    assert!(f.launch(&mut stdin), "the app never started");
    let before = f.last_frame();
    let got = f.stranger();
    assert_eq!(got, Ok(0), "EOF and nothing back");
    assert!(!f.frame_came_in(before), "no frame for a stranger");
    let stderr = f.finish(child, DRAIN);
    assert!(
        !stderr.contains("askpass"),
        "nothing logged about it: {stderr}"
    );
}

#[test]
fn a_stranger_with_no_app_in_front_is_closed_without_a_reply() {
    let f = Fixture::new("stranger-none", &[]);
    let child = f.spawn();
    let before = f.wait_frame(None);
    assert!(before.is_some(), "the shell drew no frame");
    let got = f.stranger();
    assert_eq!(got, Ok(0), "EOF and nothing back");
    assert!(!f.frame_came_in(before), "no frame for a stranger");
    let stderr = f.finish(child, None, "wait 300");
    assert!(
        !stderr.contains("askpass"),
        "nothing logged about it: {stderr}"
    );
}

#[test]
fn a_wrong_password_keeps_the_question_then_the_right_one_answers() {
    let f = Fixture::new("wrong", &[("rootprobe", "Root Probe")]);
    let mut child = f.spawn();
    let mut stdin = child.stdin.take().unwrap();
    assert!(f.launch(&mut stdin), "the probe never started");
    let prompt = f.ask();
    assert!(prompt.is_some(), "no prompt frame after the ask");
    f.keys(&mut stdin, &["wait 600", "type wrongpass", "enter"]);
    let until = Instant::now() + Duration::from_secs(2);
    while f.read("log").lines().count() < 1 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(f.read("log").lines().count(), 1, "the checker ran once");
    assert!(!f.exists("reply"), "sudo is still waiting");
    assert_eq!(f.read("calls").lines().count(), 1, "still one question");
    f.keys(&mut stdin, &["wait 600", "type hunter22", "enter"]);
    assert!(f.wait_for("reply"), "sudo never got the right one");
    let stderr = f.finish(child, Some(stdin), DRAIN);

    assert_eq!(f.read("reply"), "hunter22");
    assert_eq!(f.read("log").lines().count(), 2, "checked twice");
    assert!(
        stderr.contains("Root Probe exited with status 0 after"),
        "{stderr}"
    );
}
