//! `gh0st-askpass`: the program sudo runs for an app that asks for root
//! (docs/isolation.md, Root apps). It asks the shell, which asks the user
//! on its own screen, then prints the password for sudo. Exit 1 is no.

use gh0st_core::askpass;
use std::io::Write;
use std::path::PathBuf;

pub fn main(args: &[String]) -> i32 {
    let prompt = args.first().map(String::as_str).unwrap_or("");
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        // SAFETY: getuid has no failure mode.
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
    let fd = match askpass::connect(&askpass::socket_path(&runtime)) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("gh0st-askpass: the shell isn't asking: {e}");
            return 1;
        }
    };
    if !askpass::send(&fd, &askpass::ask(prompt)) {
        return 1;
    }
    let mut buf = [0u8; askpass::MAX_MSG];
    // The shell reads this process's binary first, which it can't once the
    // door below is shut.
    let n = askpass::recv(&fd, &mut buf).unwrap_or(0);
    if &buf[..n] != askpass::GO {
        return 1;
    }
    // Then no other process of the user may attach to this one or read its
    // memory while it holds the password, and none attached before.
    #[cfg(target_os = "linux")]
    // SAFETY: prctl with these arguments only clears a flag of this process.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
    }
    let me = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    if cfg!(target_os = "linux") && !askpass::untraced(&me) {
        eprintln!("gh0st-askpass: something traces this process");
        return 1;
    }
    if !askpass::send(&fd, askpass::READY) {
        return 1;
    }
    let n = askpass::recv(&fd, &mut buf).unwrap_or(0);
    let code = match askpass::password(&buf[..n]) {
        Some(pw) => {
            let mut out = Vec::with_capacity(pw.len() + 1);
            out.extend_from_slice(pw);
            out.push(b'\n');
            let ok = std::io::stdout().lock().write_all(&out).is_ok();
            gh0st_core::password::wipe(&mut out);
            if ok { 0 } else { 1 }
        }
        None => 1,
    };
    gh0st_core::password::wipe(&mut buf);
    code
}
