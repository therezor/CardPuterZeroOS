//! The real helper over the real link, when GH0ST_MEDIAD_TEST names a
//! build of gh0st-mediad (the image container has one): a picture whose
//! name holds a tab, a % and bytes that are not UTF-8 still opens.

use gh0st_media::helper::{Helper, Source};
use gh0st_media::proto::{Cmd, Event};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

#[test]
fn an_odd_name_reaches_the_helper_whole() {
    let Some(exe) = std::env::var_os("GH0ST_MEDIAD_TEST") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("gh0st-mediad-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(std::ffi::OsStr::from_bytes(b"a\tb%41 \xff.ppm"));
    let mut ppm = b"P6\n4 3\n255\n".to_vec();
    ppm.extend_from_slice(&[200u8; 36]);
    std::fs::write(&path, ppm).unwrap();
    let h = Helper::start(&Source::Exec(vec![exe.to_string_lossy().into_owned()])).unwrap();
    h.send(&Cmd::Hello);
    h.send(&Cmd::Probe {
        id: 7,
        path: PathBuf::from(&path),
    });
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut got = None;
    while std::time::Instant::now() < end && got.is_none() {
        let mut fds = [libc::pollfd {
            fd: h.fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        // SAFETY: one pollfd on the helper's link.
        unsafe { libc::poll(fds.as_mut_ptr(), 1, 200) };
        while let Some(e) = h.recv() {
            if let Event::Info { id: 7, info } = &e {
                got = Some((info.w, info.h));
            }
            if let Event::Fail { id: 7, why } = &e {
                panic!("the helper failed it: {why:?}");
            }
        }
    }
    assert_eq!(got, Some((4, 3)));
    std::fs::remove_dir_all(&dir).unwrap();
}
