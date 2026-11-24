//! FTP, FTPS and WebDAV through curl, one run a call. The password goes to
//! curl on stdin (`-K -`), never in its arguments, where any process could
//! read it.

use super::parse::{self, pct};
use super::{Fs, Prog, Pump, Res, run};
use crate::engine::Ask;
use crate::model::Entry;
use crate::servers::{Proto, Server};
use std::path::Path;
use std::process::Command;

pub struct Curl {
    s: Server,
    password: Option<String>,
    /// The password was asked once this session: a refusal asks again.
    asked: bool,
}

/// What a run of curl ended with.
struct Out {
    code: i32,
    body: Vec<u8>,
    err: String,
    /// The HTTP status, for WebDAV.
    http: u32,
}

impl Curl {
    pub fn new(s: Server) -> Curl {
        Curl {
            s,
            password: None,
            asked: false,
        }
    }

    fn dav(&self) -> bool {
        matches!(self.s.proto, Proto::Dav | Proto::Davs)
    }

    fn base(&self) -> String {
        let scheme = match self.s.proto {
            Proto::Ftp | Proto::Ftps => "ftp",
            Proto::Davs => "https",
            Proto::Dav => "http",
            Proto::Sftp => "sftp",
        };
        let host = if self.s.host.contains(':') {
            format!("[{}]", self.s.host)
        } else {
            self.s.host.clone()
        };
        format!("{scheme}://{host}:{}", self.s.port)
    }

    fn url(&self, path: &str) -> String {
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        format!("{}{}", self.base(), pct(&path))
    }

    fn dir_url(&self, path: &str) -> String {
        let u = self.url(path);
        if u.ends_with('/') { u } else { format!("{u}/") }
    }

    /// curl's config on stdin: the user and password, quoted.
    fn config(&self) -> String {
        if self.s.user.is_empty() {
            return String::new();
        }
        let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        format!(
            "user = \"{}:{}\"\n",
            q(&self.s.user),
            q(self.password.as_deref().unwrap_or(""))
        )
    }

    fn ensure_password(&mut self, p: &mut Pump, retry: bool) -> Res<()> {
        if self.s.user.is_empty() || (self.password.is_some() && !retry) {
            return Ok(());
        }
        let id = p.current.as_ref().map_or(0, |c| c.0);
        let pw = p
            .ask(
                id,
                Ask::Password {
                    server: self.s.name.clone(),
                    retry,
                },
            )
            .ok_or("Not connected")?;
        self.password = Some(pw);
        self.asked = true;
        Ok(())
    }

    fn denied(&self, o: &Out) -> bool {
        if self.dav() {
            o.http == 401
        } else {
            o.code == 67
        }
    }

    /// Runs curl with `args`, asking for the password first, and once more
    /// after a refusal.
    fn go(
        &mut self,
        p: &mut Pump,
        args: &[String],
        tick: &mut dyn FnMut(&mut Pump, &str) -> Res<()>,
    ) -> Res<Out> {
        self.ensure_password(p, false)?;
        for attempt in 0..2 {
            let o = self.once(p, args, tick)?;
            if self.denied(&o) && attempt == 0 && !self.s.user.is_empty() {
                self.ensure_password(p, true)?;
                continue;
            }
            return Ok(o);
        }
        Err("The server didn't take the user or password".into())
    }

    fn once(
        &mut self,
        p: &mut Pump,
        args: &[String],
        tick: &mut dyn FnMut(&mut Pump, &str) -> Res<()>,
    ) -> Res<Out> {
        let mut cmd = Command::new("curl");
        cmd.args(["-K", "-", "--connect-timeout", "15", "-S"]);
        if self.dav() {
            cmd.args(["--anyauth", "-w", "\n%{http_code}"]);
        }
        if self.s.proto == Proto::Ftps {
            cmd.arg("--ssl-reqd");
        }
        cmd.args(args);
        let id = p.current.as_ref().map(|c| c.0);
        let (code, mut body, err) = run(p, &mut cmd, Some(self.config().as_bytes()), id, tick)?;
        let mut http = 0;
        if self.dav() {
            if let Some(i) = body.iter().rposition(|&b| b == b'\n') {
                http = String::from_utf8_lossy(&body[i + 1..])
                    .trim()
                    .parse()
                    .unwrap_or(0);
                body.truncate(i);
            }
        }
        Ok(Out {
            code,
            body,
            err,
            http,
        })
    }

    fn fail(&self, o: &Out) -> String {
        if self.dav() {
            match o.http {
                401 | 403 => return "The server says no permission".into(),
                404 => return "It isn't there any more".into(),
                405 | 409 => return "The server refused".into(),
                507 => return "The server is full".into(),
                _ => {}
            }
        }
        match o.code {
            6 => format!("Can't find {}", self.s.host),
            7 => format!("{} refused the connection", self.s.host),
            28 => format!("{} didn't answer", self.s.host),
            9 | 78 => "It isn't there any more".into(),
            35 | 60 => "The server's certificate isn't trusted".into(),
            64 => "The server has no TLS".into(),
            67 => "The server didn't take the user or password".into(),
            _ => o
                .err
                .lines()
                .rev()
                .find(|l| l.starts_with("curl:"))
                .map(|l| l.trim_start_matches("curl: ").to_string())
                .unwrap_or_else(|| format!("curl failed ({})", o.code)),
        }
    }

    fn ok(&self, o: &Out) -> bool {
        o.code == 0 && (!self.dav() || (200..300).contains(&o.http))
    }

    /// An FTP command before an empty transfer: MKD, DELE, RMD, RNFR/RNTO.
    fn quote(&mut self, p: &mut Pump, cmds: &[String]) -> Res<()> {
        if cmds.iter().any(|c| c.contains(['\r', '\n'])) {
            return Err("A name can't have a line break here".into());
        }
        let mut args = Vec::new();
        for c in cmds {
            args.push("-Q".to_string());
            args.push(c.clone());
        }
        args.extend(["-o".into(), "/dev/null".into(), format!("{}/", self.base())]);
        let o = self.go(p, &args, &mut |_, _| Ok(()))?;
        if self.ok(&o) {
            Ok(())
        } else {
            Err(self.fail(&o))
        }
    }

    fn propfind(
        &mut self,
        p: &mut Pump,
        path: &str,
        depth: u8,
    ) -> Res<Option<Vec<(String, Entry)>>> {
        const BODY: &str = "<?xml version=\"1.0\"?><d:propfind xmlns:d=\"DAV:\"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>";
        let url = if depth == 1 {
            self.dir_url(path)
        } else {
            self.url(path)
        };
        let args: Vec<String> = [
            "-X",
            "PROPFIND",
            "-H",
            &format!("Depth: {depth}"),
            "-H",
            "Content-Type: application/xml",
            "--data",
            BODY,
            &url,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let o = self.go(p, &args, &mut |_, _| Ok(()))?;
        if o.http == 404 {
            return Ok(None);
        }
        if o.code != 0 || o.http != 207 {
            return Err(self.fail(&o));
        }
        Ok(Some(parse::propfind(&String::from_utf8_lossy(&o.body))))
    }
}

/// Reads curl's progress bar, "###  42.0%", for a percent.
fn percent(piece: &str) -> Option<f64> {
    piece
        .rsplit(['\r', '\n', ' '])
        .find_map(|w| w.strip_suffix('%').and_then(|n| n.parse::<f64>().ok()))
}

impl Fs for Curl {
    fn list(&mut self, p: &mut Pump, path: &str) -> Res<Vec<Entry>> {
        if self.dav() {
            let want = path.trim_end_matches('/');
            let v = self
                .propfind(p, path, 1)?
                .ok_or("It isn't there any more")?;
            return Ok(v
                .into_iter()
                .filter(|(href, e)| href.trim_end_matches('/') != want && !e.name.is_empty())
                .map(|(_, e)| e)
                .collect());
        }
        let url = self.dir_url(path);
        let o = self.go(
            p,
            &["-X".into(), "MLSD".into(), url.clone()],
            &mut |_, _| Ok(()),
        )?;
        if o.code == 0 {
            return Ok(String::from_utf8_lossy(&o.body)
                .lines()
                .filter_map(parse::mlsd_line)
                .collect());
        }
        // Servers without MLSD: the classic listing.
        let o = self.go(p, &[url], &mut |_, _| Ok(()))?;
        if o.code != 0 {
            return Err(self.fail(&o));
        }
        Ok(String::from_utf8_lossy(&o.body)
            .lines()
            .filter_map(parse::ls_line)
            .collect())
    }

    fn stat(&mut self, p: &mut Pump, path: &str) -> Res<Option<Entry>> {
        let t = path.trim_end_matches('/');
        if t.is_empty() {
            return Ok(Some(Entry::dir("/")));
        }
        if self.dav() {
            return Ok(self
                .propfind(p, path, 0)?
                .and_then(|v| v.into_iter().next())
                .map(|(_, e)| e));
        }
        let (dir, name) = t.rsplit_once('/').unwrap_or(("", t));
        let dir = if dir.is_empty() { "/" } else { dir };
        match self.list(p, dir) {
            Ok(v) => Ok(v.into_iter().find(|e| e.name == name)),
            Err(e) if e == "It isn't there any more" => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn get(&mut self, p: &mut Pump, path: &str, to: &Path, prog: &mut Prog) -> Res<()> {
        let mut args = vec!["-o".to_string(), to.to_string_lossy().into_owned()];
        if self.dav() {
            args.push("-f".into());
        }
        args.push(self.url(path));
        let mut seen = 0u64;
        let to_owned = to.to_path_buf();
        let o = self.go(p, &args, &mut |p, _| {
            let now = std::fs::metadata(&to_owned).map(|m| m.len()).unwrap_or(0);
            if now > seen {
                let d = now - seen;
                seen = now;
                prog.add(p, d)?;
            } else if p.cancelled(prog.id) {
                return Err(super::STOPPED.into());
            }
            Ok(())
        });
        let o = match o {
            Ok(o) => o,
            Err(e) => {
                let _ = std::fs::remove_file(to);
                return Err(e);
            }
        };
        if !self.ok(&o) {
            let _ = std::fs::remove_file(to);
            return Err(self.fail(&o));
        }
        let end = std::fs::metadata(to).map(|m| m.len()).unwrap_or(0);
        if end > seen {
            prog.add(p, end - seen)?;
        }
        Ok(())
    }

    fn put(&mut self, p: &mut Pump, from: &Path, path: &str, prog: &mut Prog) -> Res<()> {
        let size = std::fs::metadata(from).map(|m| m.len()).unwrap_or(0);
        let mut args = vec![
            "-#".to_string(),
            "-T".into(),
            from.to_string_lossy().into_owned(),
        ];
        if self.dav() {
            args.push("-f".into());
        }
        args.push(self.url(path));
        let mut sent = 0u64;
        let o = self.go(p, &args, &mut |p, piece| {
            if let Some(pc) = percent(piece) {
                let now = (size as f64 * pc / 100.0) as u64;
                if now > sent {
                    let d = now - sent;
                    sent = now;
                    return prog.add(p, d);
                }
            }
            if p.cancelled(prog.id) {
                return Err(super::STOPPED.into());
            }
            Ok(())
        })?;
        if !self.ok(&o) {
            return Err(self.fail(&o));
        }
        if size > sent {
            prog.add(p, size - sent)?;
        }
        Ok(())
    }

    fn mkdir(&mut self, p: &mut Pump, path: &str) -> Res<()> {
        if self.dav() {
            let url = self.dir_url(path);
            let o = self.go(p, &["-X".into(), "MKCOL".into(), url], &mut |_, _| Ok(()))?;
            return if self.ok(&o) {
                Ok(())
            } else {
                Err(self.fail(&o))
            };
        }
        self.quote(p, &[format!("MKD {path}")])
    }

    fn remove(&mut self, p: &mut Pump, path: &str, dir: bool) -> Res<()> {
        if self.dav() {
            let url = if dir {
                self.dir_url(path)
            } else {
                self.url(path)
            };
            let o = self.go(p, &["-X".into(), "DELETE".into(), url], &mut |_, _| Ok(()))?;
            return if self.ok(&o) {
                Ok(())
            } else {
                Err(self.fail(&o))
            };
        }
        self.quote(p, &[format!("{} {path}", if dir { "RMD" } else { "DELE" })])
    }

    fn rename(&mut self, p: &mut Pump, from: &str, to: &str) -> Res<()> {
        if self.dav() {
            let args = vec![
                "-X".to_string(),
                "MOVE".into(),
                "-H".into(),
                format!("Destination: {}", self.url(to)),
                "-H".into(),
                "Overwrite: F".into(),
                self.url(from),
            ];
            let o = self.go(p, &args, &mut |_, _| Ok(()))?;
            return if self.ok(&o) {
                Ok(())
            } else {
                Err(self.fail(&o))
            };
        }
        self.quote(p, &[format!("RNFR {from}"), format!("RNTO {to}")])
    }

    fn remove_tree(&mut self, p: &mut Pump, path: &str) -> Option<Res<()>> {
        // WebDAV deletes a whole collection in one call.
        self.dav().then(|| self.remove(p, path, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_config() {
        let mut s = Server::new(Proto::Davs);
        s.host = "fe80::1".into();
        s.user = "me \"x\"".into();
        let mut c = Curl::new(s);
        c.password = Some("p\\w".into());
        assert_eq!(c.url("/a b/c#d"), "https://[fe80::1]:443/a%20b/c%23d");
        assert_eq!(c.dir_url("/a"), "https://[fe80::1]:443/a/");
        assert_eq!(c.config(), "user = \"me \\\"x\\\":p\\\\w\"\n");
    }

    #[test]
    fn progress_reads_the_last_percent() {
        assert_eq!(percent("\r####      12.5%\r#####     23.0%"), Some(23.0));
        assert_eq!(percent("curl: (7) refused"), None);
    }
}
