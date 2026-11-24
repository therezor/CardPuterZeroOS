//! SFTP version 3 over `ssh -s sftp`: our own client, a request at a time.
//! ssh does the connection, the keys and the known hosts; its questions
//! come back through askpass. `GH0ST_FILES_SFTP` runs an sftp-server
//! straight instead of ssh, as the tests do.

use super::local::{detach, words};
use super::{Fs, Prog, Pump, Res, nonblock};
use crate::model::{Entry, Kind};
use crate::servers::Server;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

const INIT: u8 = 1;
const VERSION: u8 = 2;
const OPEN: u8 = 3;
const CLOSE: u8 = 4;
const READ: u8 = 5;
const WRITE: u8 = 6;
const LSTAT: u8 = 7;
const OPENDIR: u8 = 11;
const READDIR: u8 = 12;
const REMOVE: u8 = 13;
const MKDIR: u8 = 14;
const RMDIR: u8 = 15;
const REALPATH: u8 = 16;
const STAT: u8 = 17;
const RENAME: u8 = 18;
const READLINK: u8 = 19;
const STATUS: u8 = 101;
const HANDLE: u8 = 102;
const DATA: u8 = 103;
const NAME: u8 = 104;
const ATTRS: u8 = 105;

const FX_EOF: u32 = 1;
const FX_NO_SUCH_FILE: u32 = 2;
const FX_PERMISSION_DENIED: u32 = 3;

const READ_FLAG: u32 = 1;
const WRITE_FLAG: u32 = 2;
const CREAT: u32 = 8;
const TRUNC: u32 = 0x10;

const A_SIZE: u32 = 1;
const A_UIDGID: u32 = 2;
const A_PERM: u32 = 4;
const A_TIME: u32 = 8;
const A_EXT: u32 = 0x8000_0000;

/// What a request carries: 32 KB fits every server.
const CHUNK: u32 = 32 * 1024;
/// A packet past this is not a server we can talk to.
const MAX_PACKET: usize = 1 << 20;

pub struct Sftp {
    child: Child,
    inp: ChildStdin,
    out: ChildStdout,
    err: std::process::ChildStderr,
    buf: Vec<u8>,
    errs: String,
    next: u32,
    dead: bool,
}

/// A packet's body, read in order.
struct Rd<'a>(&'a [u8]);

impl Rd<'_> {
    fn u32(&mut self) -> Res<u32> {
        let b = self.0.get(..4).ok_or("short packet")?;
        let v = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        self.0 = &self.0[4..];
        Ok(v)
    }
    fn u64(&mut self) -> Res<u64> {
        Ok(((self.u32()? as u64) << 32) | self.u32()? as u64)
    }
    fn bytes(&mut self) -> Res<&[u8]> {
        let n = self.u32()? as usize;
        let b = self.0.get(..n).ok_or("short packet")?;
        self.0 = &self.0[n..];
        Ok(b)
    }
    fn string(&mut self) -> Res<String> {
        Ok(String::from_utf8_lossy(self.bytes()?).into_owned())
    }
    /// Attributes, as an entry without a name.
    fn attrs(&mut self) -> Res<Entry> {
        let flags = self.u32()?;
        let mut e = Entry::file("", 0);
        e.mode = 0;
        if flags & A_SIZE != 0 {
            e.size = self.u64()?;
        }
        if flags & A_UIDGID != 0 {
            self.u32()?;
            self.u32()?;
        }
        let mut perm = None;
        if flags & A_PERM != 0 {
            perm = Some(self.u32()?);
        }
        if flags & A_TIME != 0 {
            self.u32()?;
            e.mtime = self.u32()? as i64;
        }
        if flags & A_EXT != 0 {
            for _ in 0..self.u32()? {
                self.bytes()?;
                self.bytes()?;
            }
        }
        if let Some(p) = perm {
            e.mode = p & 0o7777;
            e.kind = match p & 0o170_000 {
                0o040_000 => Kind::Dir,
                0o120_000 => Kind::Link,
                0o100_000 => Kind::File,
                0 => Kind::File,
                _ => Kind::Other,
            };
        }
        if e.kind.is_dir() {
            e.size = 0;
        }
        Ok(e)
    }
}

fn put_str(v: &mut Vec<u8>, s: &[u8]) {
    v.extend_from_slice(&(s.len() as u32).to_be_bytes());
    v.extend_from_slice(s);
}

/// Why ssh stopped, from what it said.
fn ssh_words(err: &str, server: &Server) -> String {
    let e = err.to_ascii_lowercase();
    if e.contains("remote host identification has changed") {
        return "The server's key changed since last time. Someone may be in the way, so Files won't connect.".into();
    }
    if e.contains("host key verification failed") {
        return "The server's key wasn't trusted".into();
    }
    if e.contains("permission denied") || e.contains("too many authentication") {
        return "The server didn't take the user or password".into();
    }
    if e.contains("could not resolve") || e.contains("name or service not known") {
        return format!("Can't find {}", server.host);
    }
    if e.contains("connection refused") {
        return format!("{} refused the connection", server.host);
    }
    if e.contains("timed out") {
        return format!("{} didn't answer", server.host);
    }
    if e.contains("subsystem request failed") {
        return "The server has no SFTP".into();
    }
    match err.lines().rev().find(|l| !l.trim().is_empty()) {
        Some(l) => l.trim().to_string(),
        None => "The connection closed".into(),
    }
}

fn status_words(code: u32, msg: &str) -> String {
    match code {
        FX_NO_SUCH_FILE => "It isn't there any more".into(),
        FX_PERMISSION_DENIED => "The server says no permission".into(),
        _ if !msg.is_empty() => format!("The server said: {msg}"),
        _ => "The server refused".into(),
    }
}

impl Sftp {
    /// Opens a session: ssh, its questions through askpass, then the SFTP
    /// handshake.
    pub fn connect(p: &mut Pump, s: &Server, ssh_dir: &Path) -> Res<Sftp> {
        let mut cmd = match std::env::var_os("GH0ST_FILES_SFTP") {
            Some(server) => Command::new(server),
            None => {
                let _ = std::fs::create_dir_all(ssh_dir);
                let askpass = p.askpass_path().ok_or("No askpass socket")?.to_path_buf();
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                let opt = |k: &str, v: &dyn AsRef<std::ffi::OsStr>| {
                    let mut o = std::ffi::OsString::from(format!("{k}="));
                    o.push(v.as_ref());
                    o
                };
                let mut c = Command::new("ssh");
                c.arg("-F").arg("/dev/null");
                for (k, v) in [
                    ("BatchMode", "no"),
                    ("StrictHostKeyChecking", "ask"),
                    ("GlobalKnownHostsFile", "/dev/null"),
                    ("IdentitiesOnly", "yes"),
                    ("ConnectTimeout", "15"),
                    ("NumberOfPasswordPrompts", "2"),
                    ("LogLevel", "ERROR"),
                    ("UpdateHostKeys", "no"),
                ] {
                    c.arg("-o").arg(opt(k, &v));
                }
                c.arg("-o")
                    .arg(opt("UserKnownHostsFile", &ssh_dir.join("known_hosts")))
                    .arg("-o")
                    .arg(opt("IdentityFile", &ssh_dir.join("id_ed25519")))
                    .arg("-p")
                    .arg(s.port.to_string());
                if !s.user.is_empty() {
                    c.arg("-l").arg(&s.user);
                }
                c.arg("-s").arg("--").arg(&s.host).arg("sftp");
                c.env("SSH_ASKPASS", exe)
                    .env("SSH_ASKPASS_REQUIRE", "force")
                    .env(super::askpass::ENV, askpass)
                    .env_remove("DISPLAY");
                detach(&mut c);
                c
            }
        };
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Can't run ssh: {}", words(&e)))?;
        let (Some(inp), Some(out), Some(err)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err("ssh has no pipes".into());
        };
        nonblock(out.as_raw_fd());
        nonblock(err.as_raw_fd());
        let mut x = Sftp {
            child,
            inp,
            out,
            err,
            buf: Vec::new(),
            errs: String::new(),
            next: 0,
            dead: false,
        };
        let mut init = vec![INIT];
        init.extend_from_slice(&3u32.to_be_bytes());
        x.send(&init).map_err(|_| ssh_words(&x.errs, s))?;
        match x.packet(p) {
            Ok((VERSION, _)) => Ok(x),
            Ok(_) => Err("The server doesn't speak SFTP 3".into()),
            Err(_) => {
                let _ = x.child.kill();
                let _ = x.child.wait();
                // The last of what ssh said.
                x.drain_err();
                Err(ssh_words(&x.errs, s))
            }
        }
    }

    pub fn alive(&mut self) -> bool {
        !self.dead && matches!(self.child.try_wait(), Ok(None))
    }

    fn send(&mut self, body: &[u8]) -> Res<()> {
        let mut v = (body.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(body);
        // Blocking: sftp-server reads as we write, and we wait for each
        // answer before the next request.
        let r = self.inp.write_all(&v).and_then(|_| self.inp.flush());
        if r.is_err() {
            self.dead = true;
            return Err("The connection closed".into());
        }
        Ok(())
    }

    fn drain_err(&mut self) {
        let mut b = [0u8; 4096];
        while let Ok(n) = self.err.read(&mut b) {
            if n == 0 {
                break;
            }
            if self.errs.len() < 8192 {
                self.errs.push_str(&String::from_utf8_lossy(&b[..n]));
            }
        }
    }

    /// The next whole packet: its type and body.
    fn packet(&mut self, p: &mut Pump) -> Res<(u8, Vec<u8>)> {
        let mut b = [0u8; 65536];
        loop {
            if self.buf.len() >= 4 {
                let n = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]])
                    as usize;
                if n == 0 || n > MAX_PACKET {
                    self.dead = true;
                    return Err("The server sent nonsense".into());
                }
                if self.buf.len() >= 4 + n {
                    let pkt: Vec<u8> = self.buf.drain(..4 + n).skip(4).collect();
                    return Ok((pkt[0], pkt[1..].to_vec()));
                }
            }
            match self.out.read(&mut b) {
                Ok(0) => {
                    self.dead = true;
                    self.drain_err();
                    return Err("The connection closed".into());
                }
                Ok(n) => {
                    self.buf.extend_from_slice(&b[..n]);
                    continue;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.dead = true;
                    return Err("The connection closed".into());
                }
            }
            self.drain_err();
            let fds = [
                (self.out.as_raw_fd(), libc::POLLIN),
                (self.err.as_raw_fd(), libc::POLLIN),
            ];
            p.wait(&fds, -1);
            if p.cancelled(p.current.as_ref().map_or(0, |c| c.0)) && self.buf.is_empty() {
                // Waiting on a server that may never answer: give up the
                // session with the request.
                let _ = self.child.kill();
                self.dead = true;
                return Err(super::STOPPED.into());
            }
        }
    }

    /// Sends a request and reads its answer.
    fn call(&mut self, p: &mut Pump, ty: u8, args: &[u8]) -> Res<(u8, Vec<u8>)> {
        if self.dead {
            return Err("The connection closed".into());
        }
        self.next = self.next.wrapping_add(1);
        let id = self.next;
        let mut body = vec![ty];
        body.extend_from_slice(&id.to_be_bytes());
        body.extend_from_slice(args);
        self.send(&body)?;
        loop {
            let (t, b) = self.packet(p)?;
            let mut r = Rd(&b);
            if r.u32()? == id {
                return Ok((t, r.0.to_vec()));
            }
        }
    }

    /// A request answered with a status alone: Ok, or why not.
    fn ok(&mut self, p: &mut Pump, ty: u8, args: &[u8]) -> Res<()> {
        let (t, b) = self.call(p, ty, args)?;
        if t != STATUS {
            return Err("The server answered oddly".into());
        }
        let mut r = Rd(&b);
        let code = r.u32()?;
        if code == 0 {
            return Ok(());
        }
        Err(status_words(code, &r.string().unwrap_or_default()))
    }

    fn handle(&mut self, p: &mut Pump, ty: u8, args: &[u8]) -> Res<Vec<u8>> {
        let (t, b) = self.call(p, ty, args)?;
        let mut r = Rd(&b);
        match t {
            HANDLE => Ok(r.bytes()?.to_vec()),
            STATUS => {
                let code = r.u32()?;
                Err(status_words(code, &r.string().unwrap_or_default()))
            }
            _ => Err("The server answered oddly".into()),
        }
    }

    fn close(&mut self, p: &mut Pump, h: &[u8]) {
        let mut a = Vec::new();
        put_str(&mut a, h);
        let _ = self.ok(p, CLOSE, &a);
    }

    fn path_arg(path: &str) -> Vec<u8> {
        let mut a = Vec::new();
        put_str(&mut a, path.as_bytes());
        a
    }

    fn attrs_of(&mut self, p: &mut Pump, ty: u8, path: &str) -> Res<Option<Entry>> {
        let (t, b) = self.call(p, ty, &Self::path_arg(path))?;
        let mut r = Rd(&b);
        match t {
            ATTRS => Ok(Some(r.attrs()?)),
            STATUS => match r.u32()? {
                FX_NO_SUCH_FILE => Ok(None),
                c => Err(status_words(c, &r.string().unwrap_or_default())),
            },
            _ => Err("The server answered oddly".into()),
        }
    }

    fn realpath(&mut self, p: &mut Pump, path: &str) -> Res<String> {
        let (t, b) = self.call(p, REALPATH, &Self::path_arg(path))?;
        let mut r = Rd(&b);
        if t != NAME || r.u32()? < 1 {
            return Err("The server has no home folder".into());
        }
        r.string()
    }
}

impl Drop for Sftp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn name_of(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

impl Fs for Sftp {
    fn home(&mut self, p: &mut Pump) -> Res<String> {
        self.realpath(p, ".")
    }

    fn list(&mut self, p: &mut Pump, path: &str) -> Res<Vec<Entry>> {
        let h = self.handle(p, OPENDIR, &Self::path_arg(path))?;
        let mut out = Vec::new();
        let mut harg = Vec::new();
        put_str(&mut harg, &h);
        let r = loop {
            let (t, b) = match self.call(p, READDIR, &harg) {
                Ok(x) => x,
                Err(e) => break Err(e),
            };
            let mut r = Rd(&b);
            match t {
                NAME => {
                    for _ in 0..r.u32()? {
                        let name = r.string()?;
                        let _long = r.bytes()?;
                        let mut e = r.attrs()?;
                        if name == "." || name == ".." {
                            continue;
                        }
                        e.name = name;
                        out.push(e);
                    }
                }
                STATUS => match r.u32()? {
                    FX_EOF => break Ok(()),
                    c => break Err(status_words(c, &r.string().unwrap_or_default())),
                },
                _ => break Err("The server answered oddly".into()),
            }
            if out.len() > crate::model::MAX_ENTRIES {
                break Ok(());
            }
        };
        self.close(p, &h);
        r?;
        // Where links point, and whether to a folder.
        for e in out.iter_mut().filter(|e| e.kind == Kind::Link) {
            let full = join(path, &e.name);
            if let Ok(Some(t)) = self.attrs_of(p, STAT, &full) {
                if t.kind.is_dir() {
                    e.kind = Kind::LinkDir;
                } else {
                    e.size = t.size;
                }
            }
            if let Ok((NAME, b)) = self.call(p, READLINK, &Self::path_arg(&full)) {
                let mut r = Rd(&b);
                if r.u32().unwrap_or(0) > 0 {
                    e.link = r.string().ok();
                }
            }
        }
        Ok(out)
    }

    fn stat(&mut self, p: &mut Pump, path: &str) -> Res<Option<Entry>> {
        let Some(mut e) = self.attrs_of(p, LSTAT, path)? else {
            return Ok(None);
        };
        if e.kind == Kind::Link {
            if let Ok(Some(t)) = self.attrs_of(p, STAT, path) {
                e.kind = if t.kind.is_dir() {
                    Kind::LinkDir
                } else {
                    Kind::Link
                };
                e.size = t.size;
            }
        }
        e.name = name_of(path);
        Ok(Some(e))
    }

    fn get(&mut self, p: &mut Pump, path: &str, to: &Path, prog: &mut Prog) -> Res<()> {
        let mut a = Self::path_arg(path);
        a.extend_from_slice(&READ_FLAG.to_be_bytes());
        a.extend_from_slice(&0u32.to_be_bytes());
        let h = self.handle(p, OPEN, &a)?;
        let r = (|| {
            let mut f = std::fs::File::create(to).map_err(|e| words(&e))?;
            let mut off = 0u64;
            loop {
                let mut a = Vec::new();
                put_str(&mut a, &h);
                a.extend_from_slice(&off.to_be_bytes());
                a.extend_from_slice(&CHUNK.to_be_bytes());
                let (t, b) = self.call(p, READ, &a)?;
                let mut r = Rd(&b);
                match t {
                    DATA => {
                        let d = r.bytes()?;
                        f.write_all(d).map_err(|e| words(&e))?;
                        off += d.len() as u64;
                        prog.add(p, d.len() as u64)?;
                    }
                    STATUS => match r.u32()? {
                        FX_EOF => break,
                        c => return Err(status_words(c, &r.string().unwrap_or_default())),
                    },
                    _ => return Err("The server answered oddly".into()),
                }
            }
            f.sync_all().map_err(|e| words(&e))
        })();
        self.close(p, &h);
        if r.is_err() {
            let _ = std::fs::remove_file(to);
        }
        r
    }

    fn put(&mut self, p: &mut Pump, from: &Path, path: &str, prog: &mut Prog) -> Res<()> {
        let mut src = std::fs::File::open(from).map_err(|e| words(&e))?;
        let mut a = Self::path_arg(path);
        a.extend_from_slice(&(WRITE_FLAG | CREAT | TRUNC).to_be_bytes());
        a.extend_from_slice(&0u32.to_be_bytes());
        let h = self.handle(p, OPEN, &a)?;
        let r = (|| {
            let mut buf = vec![0u8; CHUNK as usize];
            let mut off = 0u64;
            loop {
                let n = src.read(&mut buf).map_err(|e| words(&e))?;
                if n == 0 {
                    return Ok(());
                }
                let mut a = Vec::new();
                put_str(&mut a, &h);
                a.extend_from_slice(&off.to_be_bytes());
                put_str(&mut a, &buf[..n]);
                self.ok(p, WRITE, &a)?;
                off += n as u64;
                prog.add(p, n as u64)?;
            }
        })();
        self.close(p, &h);
        r
    }

    fn mkdir(&mut self, p: &mut Pump, path: &str) -> Res<()> {
        let mut a = Self::path_arg(path);
        a.extend_from_slice(&0u32.to_be_bytes());
        self.ok(p, MKDIR, &a)
    }

    fn remove(&mut self, p: &mut Pump, path: &str, dir: bool) -> Res<()> {
        self.ok(p, if dir { RMDIR } else { REMOVE }, &Self::path_arg(path))
    }

    fn rename(&mut self, p: &mut Pump, from: &str, to: &str) -> Res<()> {
        let mut a = Self::path_arg(from);
        put_str(&mut a, to.as_bytes());
        self.ok(p, RENAME, &a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attrs_read_kinds_and_times() {
        let mut b = Vec::new();
        b.extend_from_slice(&(A_SIZE | A_PERM | A_TIME).to_be_bytes());
        b.extend_from_slice(&5u64.to_be_bytes());
        b.extend_from_slice(&0o100_644u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&1_700_000_000u32.to_be_bytes());
        let e = Rd(&b).attrs().unwrap();
        assert_eq!(
            (e.kind, e.size, e.mode, e.mtime),
            (Kind::File, 5, 0o644, 1_700_000_000)
        );
        let mut d = Vec::new();
        d.extend_from_slice(&A_PERM.to_be_bytes());
        d.extend_from_slice(&0o040_755u32.to_be_bytes());
        assert_eq!(Rd(&d).attrs().unwrap().kind, Kind::Dir);
    }

    #[test]
    fn ssh_errors_read_as_words() {
        let s = Server {
            host: "nas.local".into(),
            ..Server::new(crate::servers::Proto::Sftp)
        };
        assert_eq!(
            ssh_words(
                "me@nas.local: Permission denied (publickey,password).\n",
                &s
            ),
            "The server didn't take the user or password"
        );
        assert!(
            ssh_words(
                "@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@",
                &s
            )
            .contains("Someone may be in the way")
        );
        assert_eq!(
            ssh_words(
                "ssh: Could not resolve hostname nas.local: Name or service not known",
                &s
            ),
            "Can't find nas.local"
        );
    }
}
