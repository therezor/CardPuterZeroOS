//! A connection over a Unix socket: non-blocking reads and writes for the
//! poll loop, plus a blocking [`Conn::call`] for a client's own replies.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use crate::address::Addr;
use crate::message::{Kind, Message};
use crate::{Error, Result, auth, bus};

/// Fds waiting for their message; past this a peer is flooding us.
const MAX_FDS: usize = 64;
/// Messages `call` set aside while it waited.
const MAX_PENDING: usize = 256;
/// Bytes queued for a peer that doesn't read; past this it is dropped.
pub const MAX_QUEUED: usize = 4 << 20;
const READ_CHUNK: usize = 16 << 10;

struct Out {
    bytes: Vec<u8>,
    sent: usize,
    fds: Vec<OwnedFd>,
}

pub struct Conn {
    fd: OwnedFd,
    auth: Option<auth::Server>,
    rbuf: Vec<u8>,
    rfds: VecDeque<OwnedFd>,
    out: VecDeque<Out>,
    queued: usize,
    pending: VecDeque<Message>,
    serial: u32,
    fds_ok: bool,
    eof: bool,
    /// Our unique name on a bus.
    pub name: Option<String>,
}

impl Conn {
    fn new(fd: OwnedFd) -> Result<Conn> {
        set_nonblock_cloexec(fd.as_raw_fd())?;
        #[cfg(target_vendor = "apple")]
        {
            let one: libc::c_int = 1;
            // SAFETY: setsockopt with an int on a socket we own.
            unsafe {
                libc::setsockopt(
                    fd.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_NOSIGPIPE,
                    (&one as *const libc::c_int).cast(),
                    4,
                )
            };
        }
        Ok(Conn {
            fd,
            auth: None,
            rbuf: Vec::new(),
            rfds: VecDeque::new(),
            out: VecDeque::new(),
            queued: 0,
            pending: VecDeque::new(),
            serial: 0,
            fds_ok: false,
            eof: false,
            name: None,
        })
    }

    /// Messages straight over a socket both ends trust, with no SASL: a
    /// socketpair between our own processes.
    pub fn raw(fd: OwnedFd) -> Result<Conn> {
        let mut c = Conn::new(fd)?;
        c.fds_ok = true;
        Ok(c)
    }

    /// The server's end of a fresh connection: SASL runs first.
    pub fn server(fd: OwnedFd, guid: &str) -> Result<Conn> {
        let uid = peer_uid(fd.as_raw_fd())?;
        let mut c = Conn::new(fd)?;
        c.auth = Some(auth::Server::new(uid, guid, true));
        Ok(c)
    }

    /// Connects, authenticates and says Hello. Blocks up to 5 s.
    pub fn open(addr: &Addr) -> Result<Conn> {
        let mut s = match addr {
            Addr::Path(p) => UnixStream::connect(p)?,
            #[cfg(target_os = "linux")]
            Addr::Abstract(name) => {
                use std::os::linux::net::SocketAddrExt;
                let a = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
                UnixStream::connect_addr(&a)?
            }
            #[cfg(not(target_os = "linux"))]
            Addr::Abstract(_) => return Err(Error::Invalid("no abstract sockets here")),
        };
        s.set_read_timeout(Some(Duration::from_secs(5)))?;
        // SAFETY: getuid cannot fail.
        let uid = unsafe { libc::getuid() };
        s.write_all(&auth::client_hello(uid, true))?;
        let mut client = auth::Client::new(true);
        let mut b = [0u8; 1];
        loop {
            if s.read(&mut b)? == 0 {
                return Err(Error::Closed);
            }
            if client.feed(b[0])? {
                break;
            }
        }
        let mut c = Conn::new(OwnedFd::from(s))?;
        c.fds_ok = client.fds;
        let reply = c.call(
            Message::call(bus::NAME, bus::PATH, bus::NAME, "Hello"),
            5000,
        )?;
        c.name = reply
            .body
            .first()
            .and_then(|v| v.as_str())
            .map(String::from);
        Ok(c)
    }

    pub fn session() -> Result<Conn> {
        Conn::open(&crate::address::session().ok_or(Error::Invalid("no session bus"))?)
    }

    pub fn system() -> Result<Conn> {
        Conn::open(&crate::address::system().ok_or(Error::Invalid("no system bus"))?)
    }

    pub fn fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    pub fn wants_write(&self) -> bool {
        !self.out.is_empty()
    }

    /// Bytes waiting to go out.
    pub fn queued(&self) -> usize {
        self.queued
    }

    pub fn closed(&self) -> bool {
        self.eof
    }

    pub fn authed(&self) -> bool {
        self.auth.is_none()
    }

    /// The peer's process: SO_PEERCRED's pid on Linux.
    pub fn peer_pid(&self) -> Option<u32> {
        peer_pid(self.fd())
    }

    /// Reads everything the socket has. Call when poll says readable.
    pub fn fill(&mut self) -> Result<()> {
        let mut chunk = vec![0u8; READ_CHUNK];
        loop {
            let (n, fds) = match recv(self.fd(), &mut chunk) {
                Ok(r) => r,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    crate::wipe(&mut chunk);
                    return Err(e.into());
                }
            };
            self.rfds.extend(fds);
            if self.rfds.len() > MAX_FDS {
                crate::wipe(&mut chunk);
                return Err(Error::Invalid("too many fds"));
            }
            if n == 0 {
                self.eof = true;
                break;
            }
            let data = &chunk[..n];
            if let Some(a) = &mut self.auth {
                let mut reply = Vec::new();
                let used = a.feed(data, &mut reply)?;
                let fds = a.fds;
                if !reply.is_empty() {
                    self.queue(reply, Vec::new());
                }
                if let Some(used) = used {
                    self.fds_ok = fds;
                    self.auth = None;
                    self.rbuf.extend_from_slice(&data[used..]);
                }
            } else {
                self.rbuf.extend_from_slice(data);
            }
            if self.rbuf.len() > crate::MAX_MESSAGE * 2 {
                crate::wipe(&mut chunk);
                return Err(Error::Invalid("peer sends faster than we read"));
            }
        }
        crate::wipe(&mut chunk);
        self.flush()
    }

    /// The next whole message, if one is in.
    pub fn recv(&mut self) -> Result<Option<Message>> {
        if let Some(m) = self.pending.pop_front() {
            return Ok(Some(m));
        }
        self.decode_next()
    }

    fn decode_next(&mut self) -> Result<Option<Message>> {
        if self.auth.is_some() {
            return Ok(None);
        }
        let Some(total) = Message::frame_len(&self.rbuf)? else {
            return Ok(None);
        };
        if self.rbuf.len() < total {
            return Ok(None);
        }
        let m = Message::decode(&self.rbuf[..total], &mut self.rfds);
        crate::wipe(&mut self.rbuf[..total]);
        self.rbuf.drain(..total);
        m.map(Some)
    }

    /// Queues a message and writes what the socket takes now. Returns
    /// the serial it went out with.
    pub fn send(&mut self, mut msg: Message) -> Result<u32> {
        if !msg.fds.is_empty() && !self.fds_ok {
            return Err(Error::Invalid("this peer takes no fds"));
        }
        self.serial = self.serial.wrapping_add(1).max(1);
        msg.serial = self.serial;
        let bytes = msg.encode(self.serial);
        // A reply may carry a secret: zero it once it is bytes.
        msg.wipe();
        let bytes = bytes?;
        let fds = std::mem::take(&mut msg.fds);
        self.queue(bytes, fds);
        self.flush()?;
        Ok(self.serial)
    }

    fn queue(&mut self, bytes: Vec<u8>, fds: Vec<OwnedFd>) {
        self.queued += bytes.len();
        self.out.push_back(Out {
            bytes,
            sent: 0,
            fds,
        });
    }

    /// Writes queued bytes until the socket would block.
    pub fn flush(&mut self) -> Result<()> {
        while let Some(o) = self.out.front_mut() {
            let fds: Vec<RawFd> = if o.sent == 0 {
                o.fds.iter().map(AsRawFd::as_raw_fd).collect()
            } else {
                Vec::new()
            };
            match send(self.fd.as_raw_fd(), &o.bytes[o.sent..], &fds) {
                Ok(n) => {
                    o.sent += n;
                    o.fds.clear();
                    if o.sent == o.bytes.len() {
                        self.queued -= o.bytes.len();
                        crate::wipe(&mut o.bytes);
                        self.out.pop_front();
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                    self.eof = true;
                    return Err(Error::Closed);
                }
                Err(e) => return Err(e.into()),
            }
        }
        if self.queued > MAX_QUEUED {
            return Err(Error::Invalid("peer does not read"));
        }
        Ok(())
    }

    /// Sends a call and waits for its reply. Anything else that arrives
    /// meanwhile waits for [`Conn::recv`].
    pub fn call(&mut self, msg: Message, timeout_ms: u64) -> Result<Message> {
        let serial = self.send(msg)?;
        let end = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            while let Some(m) = self.decode_next()? {
                if matches!(m.kind, Kind::Return | Kind::Error) && m.reply_serial == Some(serial) {
                    if m.kind == Kind::Error {
                        return Err(Error::Remote {
                            name: m.error_name.clone().unwrap_or_default(),
                            text: m.error_text(),
                        });
                    }
                    return Ok(m);
                }
                if self.pending.len() >= MAX_PENDING {
                    return Err(Error::Invalid("too many messages while waiting"));
                }
                self.pending.push_back(m);
            }
            if self.eof {
                return Err(Error::Closed);
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::Timeout);
            }
            let mut events = libc::POLLIN;
            if self.wants_write() {
                events |= libc::POLLOUT;
            }
            let mut p = libc::pollfd {
                fd: self.fd(),
                events,
                revents: 0,
            };
            let ms = left.as_millis().clamp(1, i32::MAX as u128) as i32;
            // SAFETY: poll on one valid pollfd.
            if unsafe { libc::poll(&mut p, 1, ms) } < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() != std::io::ErrorKind::Interrupted {
                    return Err(e.into());
                }
            }
            self.flush()?;
            self.fill()?;
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        crate::wipe(&mut self.rbuf);
        for o in &mut self.out {
            crate::wipe(&mut o.bytes);
        }
    }
}

fn set_nonblock_cloexec(fd: RawFd) -> std::io::Result<()> {
    // SAFETY: fcntl on a descriptor we own.
    unsafe {
        let fl = libc::fcntl(fd, libc::F_GETFL);
        if fl < 0 || libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

const CMSG_FDS: usize = 32;

fn cmsg_space() -> usize {
    // SAFETY: CMSG_SPACE is pure arithmetic.
    unsafe { libc::CMSG_SPACE((CMSG_FDS * std::mem::size_of::<RawFd>()) as u32) as usize }
}

fn recv(fd: RawFd, buf: &mut [u8]) -> std::io::Result<(usize, Vec<OwnedFd>)> {
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast(),
        iov_len: buf.len(),
    };
    // u64 keeps the control buffer aligned for cmsghdr.
    let mut ctl = vec![0u64; cmsg_space().div_ceil(8)];
    // SAFETY: a zeroed msghdr is valid; we point it at live buffers.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = ctl.as_mut_ptr().cast();
    msg.msg_controllen = (ctl.len() * 8) as _;
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC;
    #[cfg(not(target_os = "linux"))]
    let flags = libc::MSG_DONTWAIT;
    // SAFETY: recvmsg into the buffers set up above.
    let n = unsafe { libc::recvmsg(fd, &mut msg, flags) };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut fds = Vec::new();
    // SAFETY: walking the control buffer recvmsg filled, with its macros.
    unsafe {
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(c);
                let len = (*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                for i in 0..len / std::mem::size_of::<RawFd>() {
                    let raw = std::ptr::read_unaligned(data.cast::<RawFd>().add(i));
                    let owned = OwnedFd::from_raw_fd(raw);
                    libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC);
                    fds.push(owned);
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
    }
    if msg.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(std::io::Error::other("fds were cut off"));
    }
    Ok((n as usize, fds))
}

fn send(fd: RawFd, bytes: &[u8], fds: &[RawFd]) -> std::io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr() as *mut _,
        iov_len: bytes.len(),
    };
    // SAFETY: a zeroed msghdr is valid.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    let mut ctl: Vec<u64> = Vec::new();
    if !fds.is_empty() {
        let n = std::mem::size_of_val(fds);
        // SAFETY: CMSG_SPACE is arithmetic.
        let space = unsafe { libc::CMSG_SPACE(n as u32) } as usize;
        ctl = vec![0u64; space.div_ceil(8)];
        msg.msg_control = ctl.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        // SAFETY: the control buffer holds one header and `n` bytes.
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(n as u32) as _;
            std::ptr::copy_nonoverlapping(fds.as_ptr().cast::<u8>(), libc::CMSG_DATA(c), n);
        }
    }
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL;
    #[cfg(not(target_os = "linux"))]
    let flags = libc::MSG_DONTWAIT;
    // SAFETY: sendmsg from live buffers.
    let n = unsafe { libc::sendmsg(fd, &msg, flags) };
    drop(ctl);
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(n as usize)
}

/// The UID at the other end of a Unix socket.
pub fn peer_uid(fd: RawFd) -> std::io::Result<u32> {
    #[cfg(target_os = "linux")]
    {
        peer_cred(fd).map(|c| c.uid)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let (mut uid, mut gid) = (0, 0);
        // SAFETY: getpeereid fills two ints.
        if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(uid)
    }
}

pub fn peer_pid(fd: RawFd) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        peer_cred(fd).ok().map(|c| c.pid as u32)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = fd;
        None
    }
}

#[cfg(target_os = "linux")]
fn peer_cred(fd: RawFd) -> std::io::Result<libc::ucred> {
    // SAFETY: a zeroed ucred is valid.
    let mut c: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: getsockopt fills a ucred of the length we pass.
    let r = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut c as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if r != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(c)
}

/// A connected pair for tests and for links between our own processes.
pub fn pair() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let (a, b) = UnixStream::pair()?;
    // SAFETY: into_raw_fd hands over ownership.
    Ok(unsafe {
        (
            OwnedFd::from_raw_fd(a.into_raw_fd()),
            OwnedFd::from_raw_fd(b.into_raw_fd()),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    fn both() -> (Conn, Conn) {
        let (a, b) = pair().unwrap();
        (Conn::raw(a).unwrap(), Conn::raw(b).unwrap())
    }

    fn pump(c: &mut Conn) -> Message {
        for _ in 0..1000 {
            c.fill().unwrap();
            if let Some(m) = c.recv().unwrap() {
                return m;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("no message")
    }

    #[test]
    fn messages_cross_a_socketpair() {
        let (mut a, mut b) = both();
        let mut sig = Message::signal("/x", "a.b", "C").arg(Value::Bytes(vec![7; 100_000]));
        sig.sender = Some(":1.1".into());
        a.send(sig).unwrap();
        while a.wants_write() {
            b.fill().unwrap();
            a.flush().unwrap();
        }
        let m = pump(&mut b);
        assert_eq!(m.body[0].as_bytes().unwrap().len(), 100_000);
        assert_eq!(m.serial, 1);
    }

    #[test]
    fn fds_cross_with_their_message() {
        let (mut a, mut b) = both();
        let (x, _y) = pair().unwrap();
        let mut sig = Message::signal("/x", "a.b", "C").arg(Value::Fd(0));
        sig.fds.push(x);
        a.send(sig).unwrap();
        let m = pump(&mut b);
        assert_eq!(m.fds.len(), 1);
        assert_eq!(m.body[0], Value::Fd(0));
    }

    #[test]
    fn a_call_waits_for_its_own_reply() {
        let (mut a, b) = both();
        let server = std::thread::spawn(move || {
            let mut b = b;
            let call = pump(&mut b);
            let mut sig = Message::signal("/x", "a.b", "Noise");
            sig.sender = Some(":1.9".into());
            b.send(sig).unwrap();
            b.send(Message::reply(&call).arg(Value::U32(42))).unwrap();
        });
        let r = a
            .call(Message::call(":1.9", "/x", "a.b", "Ask"), 2000)
            .unwrap();
        assert_eq!(r.body[0], Value::U32(42));
        assert!(a.recv().unwrap().unwrap().is("a.b", "Noise"));
        server.join().unwrap();
    }

    #[test]
    fn a_server_authenticates_then_reads() {
        let (a, b) = pair().unwrap();
        let mut srv = Conn::server(b, "guid").unwrap();
        let mut cli = UnixStream::from(a);
        // SAFETY: getuid cannot fail.
        let uid = unsafe { libc::getuid() };
        let mut bytes = auth::client_hello(uid, true);
        bytes.extend(Message::signal("/x", "a.b", "C").encode(1).unwrap());
        cli.write_all(&bytes).unwrap();
        let m = pump(&mut srv);
        assert!(m.is("a.b", "C"));
        let mut answer = [0u8; 64];
        let n = cli.read(&mut answer).unwrap();
        assert!(answer[..n].starts_with(b"OK guid\r\n"));
    }

    #[test]
    fn junk_is_an_error_not_a_panic() {
        let (a, b) = pair().unwrap();
        let mut srv = Conn::raw(b).unwrap();
        let mut cli = UnixStream::from(a);
        cli.write_all(b"xxxxxxxxxxxxxxxxxxxxxxxx").unwrap();
        srv.fill().unwrap();
        assert!(srv.recv().is_err());
    }
}
