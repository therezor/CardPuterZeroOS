//! Watched fds, the write queue and raw keys, through the test driver.

use gh0st_app::prelude::*;
use gh0st_app::testing::Driver;
use gh0st_app::{KeyState, RawKey};
use std::cell::RefCell;
use std::os::fd::RawFd;
use std::rc::Rc;

fn pipe() -> (RawFd, RawFd) {
    let mut fds = [0; 2];
    // SAFETY: pipe(2) fills a two-int array.
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    for fd in fds {
        // SAFETY: fcntl on an fd we own.
        unsafe { libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK) };
    }
    (fds[0], fds[1])
}

fn read_all(fd: RawFd, out: &mut Vec<u8>) {
    let mut buf = [0u8; 4096];
    loop {
        // SAFETY: reads into a stack buffer.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            return;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
}

#[derive(Default)]
struct Seen {
    bytes: Vec<u8>,
    keys: Vec<(u16, KeyState)>,
}

struct Reader {
    fd: RawFd,
    out: Option<RawFd>,
    send: usize,
    watch: Option<Watch>,
    sink: Option<Watch>,
    seen: Rc<RefCell<Seen>>,
}

impl Page for Reader {
    fn title(&self) -> &str {
        "reader"
    }
    fn draw(&self, _: &mut Ui) {}
    fn start(&mut self, cx: &mut Cx) {
        self.watch = Some(cx.watch(self.fd));
        if let Some(out) = self.out {
            let w = cx.watch(out);
            cx.write(w, &vec![7u8; self.send]);
            self.sink = Some(w);
        }
    }
    fn ready(&mut self, _: Watch, _: &mut Cx) {
        read_all(self.fd, &mut self.seen.borrow_mut().bytes);
    }
    fn raw_keys(&self) -> bool {
        true
    }
    fn raw(&mut self, key: &RawKey, _: &mut Cx) {
        self.seen.borrow_mut().keys.push((key.code, key.state));
    }
}

fn reader(fd: RawFd, out: Option<RawFd>, send: usize) -> (Reader, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let page = Reader {
        fd,
        out,
        send,
        watch: None,
        sink: None,
        seen: seen.clone(),
    };
    (page, seen)
}

#[test]
fn a_watched_fd_reaches_the_page() {
    let (r, w) = pipe();
    let (page, seen) = reader(r, None, 0);
    let mut d = Driver::new(page);
    // SAFETY: writes a static buffer to our pipe.
    unsafe { libc::write(w, b"hello".as_ptr().cast(), 5) };
    d.pump(50);
    assert_eq!(seen.borrow().bytes, b"hello");
}

#[test]
fn writes_queue_until_there_is_room() {
    let (r, _) = pipe();
    let (sink_r, sink_w) = pipe();
    let total = 300_000;
    let (page, _) = reader(r, Some(sink_w), total);
    let mut d = Driver::new(page);
    let mut got = Vec::new();
    for _ in 0..200 {
        read_all(sink_r, &mut got);
        if got.len() == total {
            break;
        }
        d.pump(5);
    }
    assert_eq!(got.len(), total);
    assert!(got.iter().all(|&b| b == 7));
}

#[test]
fn raw_keys_bring_releases_and_esc_comes_from_fn_esc() {
    let (r, _) = pipe();
    let (page, seen) = reader(r, None, 0);
    let mut d = Driver::new(page);
    d.keys("ctrl+c\nfn+esc\nesc");
    assert!(!d.quit());
    assert_eq!(d.toast(), Some("Hold to exit"), "plain Esc goes back");
    let keys = &seen.borrow().keys;
    assert!(keys.contains(&(46, KeyState::Press)));
    assert!(keys.contains(&(46, KeyState::Release)));
    assert!(
        keys.contains(&(1, KeyState::Press)),
        "Fn+Esc is the Esc key"
    );
}
