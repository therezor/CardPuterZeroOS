//! Saved servers: one per line in `servers`, in the app's data folder. No
//! secret goes in it: a remembered password is a keyring item, found by
//! [`Server::attrs`].

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    Sftp,
    Ftp,
    Ftps,
    /// WebDAV over https.
    Davs,
    /// WebDAV over plain http.
    Dav,
}

impl Proto {
    pub const ALL: [Proto; 5] = [
        Proto::Sftp,
        Proto::Ftps,
        Proto::Ftp,
        Proto::Davs,
        Proto::Dav,
    ];

    /// How the server form and Places name it.
    pub fn word(self) -> &'static str {
        match self {
            Proto::Sftp => "SFTP",
            Proto::Ftp => "FTP",
            Proto::Ftps => "FTPS",
            Proto::Davs => "WebDAV",
            Proto::Dav => "WebDAV http",
        }
    }

    /// The file's and the keyring's word.
    pub fn id(self) -> &'static str {
        match self {
            Proto::Sftp => "sftp",
            Proto::Ftp => "ftp",
            Proto::Ftps => "ftps",
            Proto::Davs => "davs",
            Proto::Dav => "dav",
        }
    }

    pub fn parse(s: &str) -> Option<Proto> {
        Proto::ALL.into_iter().find(|p| p.id() == s)
    }

    pub fn port(self) -> u16 {
        match self {
            Proto::Sftp => 22,
            Proto::Ftp | Proto::Ftps => 21,
            Proto::Davs => 443,
            Proto::Dav => 80,
        }
    }

    /// Sends the password and the files where anyone on the network can
    /// read them.
    pub fn plain(self) -> bool {
        matches!(self, Proto::Ftp | Proto::Dav)
    }
}

/// The schema of the keyring items Files keeps.
pub const SCHEMA: &str = "org.gh0st.Files.Server";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    /// Unique, no tab or colon: Places shows it, and paths start with it.
    pub name: String,
    pub proto: Proto,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Where the server's side opens.
    pub path: String,
    /// The password is kept in the keyring.
    pub remember: bool,
}

impl Server {
    pub fn new(proto: Proto) -> Server {
        Server {
            name: String::new(),
            proto,
            host: String::new(),
            port: proto.port(),
            user: String::new(),
            path: "/".into(),
            remember: true,
        }
    }

    /// "nas.local" or "nas.local:2222".
    pub fn address(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == self.proto.port() {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }

    /// Takes "host", "host:port", or "[v6]:port".
    pub fn set_address(&mut self, text: &str) -> Result<(), &'static str> {
        let t = text.trim();
        let (host, port) = if let Some(rest) = t.strip_prefix('[') {
            let (h, after) = rest.split_once(']').ok_or("A ] is missing")?;
            (h, after.strip_prefix(':'))
        } else if t.matches(':').count() == 1 {
            let (h, p) = t.split_once(':').ok_or("")?;
            (h, Some(p))
        } else {
            (t, None)
        };
        if host.is_empty() || host.contains(char::is_whitespace) {
            return Err("Type a name or address");
        }
        let port = match port {
            Some(p) => p
                .parse::<u16>()
                .ok()
                .filter(|&p| p > 0)
                .ok_or("The port is 1 to 65535")?,
            None => self.proto.port(),
        };
        self.host = host.into();
        self.port = port;
        Ok(())
    }

    /// What finds its password in the keyring.
    pub fn attrs(&self) -> [(&'static str, String); 5] {
        [
            ("xdg:schema", SCHEMA.into()),
            ("protocol", self.proto.id().into()),
            ("host", self.host.clone()),
            ("port", self.port.to_string()),
            ("user", self.user.clone()),
        ]
    }

    /// The keyring item's label, as the Keyring app shows it.
    pub fn label(&self) -> String {
        format!("{} ({})", self.name, self.proto.word())
    }

    fn line(&self) -> String {
        [
            esc(&self.name),
            self.proto.id().into(),
            esc(&self.host),
            self.port.to_string(),
            esc(&self.user),
            esc(&self.path),
            if self.remember { "1" } else { "0" }.into(),
        ]
        .join("\t")
    }

    fn parse(line: &str) -> Option<Server> {
        let f: Vec<&str> = line.split('\t').collect();
        let [name, proto, host, port, user, path, remember] = f[..] else {
            return None;
        };
        Some(Server {
            name: unesc(name),
            proto: Proto::parse(proto)?,
            host: unesc(host),
            port: port.parse().ok()?,
            user: unesc(user),
            path: unesc(path),
            remember: remember == "1",
        })
    }
}

/// A server's name, once checked: unique, and no tab or colon.
pub fn name_ok(name: &str, others: &[Server]) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err("A name can't be empty");
    }
    if name.contains(['\t', '\n', ':', '/']) {
        return Err("No : or / in a server's name");
    }
    if others.iter().any(|s| s.name == name) {
        return Err("Another server has that name");
    }
    Ok(())
}

/// Tabs, newlines and backslashes, escaped so a field stays one field.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

pub fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

pub fn load(path: &Path) -> Vec<Server> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(Server::parse)
        .collect()
}

/// Writes the whole list: a temp file, then a rename, so a crash leaves the
/// old one.
pub fn save(path: &Path, servers: &[Server]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)?;
    for s in servers {
        writeln!(f, "{}", s.line())?;
    }
    f.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_round_trips_odd_names() {
        let dir = std::env::temp_dir().join(format!("gh0st-files-srv-{}", std::process::id()));
        let path = dir.join("servers");
        let mut a = Server::new(Proto::Sftp);
        a.name = "nas".into();
        a.set_address("nas.local:2222").unwrap();
        a.user = "me".into();
        let mut b = Server::new(Proto::Davs);
        b.name = "cloud\\home".into();
        b.host = "dav.example".into();
        b.path = "/remote.php/dav/files/me\tx".into();
        b.remember = false;
        save(&path, &[a.clone(), b.clone()]).unwrap();
        assert_eq!(load(&path), vec![a, b]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn addresses_take_ports_and_v6() {
        let mut s = Server::new(Proto::Sftp);
        s.set_address("nas.local").unwrap();
        assert_eq!((s.host.as_str(), s.port), ("nas.local", 22));
        s.set_address("[fe80::1]:2200").unwrap();
        assert_eq!((s.host.as_str(), s.port), ("fe80::1", 2200));
        assert_eq!(s.address(), "[fe80::1]:2200");
        assert!(s.set_address("host:0").is_err());
        assert!(s.set_address("").is_err());
        assert!(s.set_address("fe80::1").is_ok(), "a bare v6 address");
    }

    #[test]
    fn names_are_unique_and_plain() {
        let mut nas = Server::new(Proto::Ftp);
        nas.name = "nas".into();
        assert!(name_ok("nas", std::slice::from_ref(&nas)).is_err());
        assert!(name_ok("a:b", &[]).is_err());
        assert!(name_ok("home", &[nas]).is_ok());
    }
}
