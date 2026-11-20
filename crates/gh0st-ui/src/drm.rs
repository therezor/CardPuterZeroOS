//! The panel through DRM: a dumb buffer we fill, then DIRTYFB with the one
//! rectangle that changed. The panel driver (mipi-dbi over SPI) sends exactly
//! that rectangle, once, after the frame is complete.
//!
//! fbdev tears on this panel: the kernel copies changed pages on its own 50 ms
//! timer, so it can catch a frame half-written, and it always sends whole
//! rows. The panel's TE pin is not wired, so the fewer bytes per update, the
//! less a transfer can race the panel's own refresh.
//!
//! Apps draw through fbdev, which only shows while no DRM client has the
//! device open, so the shell lets go (`suspend`) before an app starts and
//! takes the panel back (`resume`) when it exits.

use crate::canvas::Canvas;
use crate::geom::Rect;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

const fn iowr(nr: u32, size: usize) -> libc::c_ulong {
    ((3u32 << 30) | ((size as u32) << 16) | (0x64 << 8) | nr) as libc::c_ulong
}

const fn io(nr: u32) -> libc::c_ulong {
    ((0x64 << 8) | nr) as libc::c_ulong
}

#[repr(C)]
#[derive(Default)]
struct CardRes {
    fb_id_ptr: u64,
    crtc_id_ptr: u64,
    connector_id_ptr: u64,
    encoder_id_ptr: u64,
    count_fbs: u32,
    count_crtcs: u32,
    count_connectors: u32,
    count_encoders: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
#[derive(Default)]
struct GetConnector {
    encoders_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    count_modes: u32,
    count_props: u32,
    count_encoders: u32,
    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ModeInfo {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,
    vrefresh: u32,
    flags: u32,
    kind: u32,
    name: [u8; 32],
}

#[repr(C)]
#[derive(Default)]
struct GetEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

#[repr(C)]
#[derive(Default)]
struct CreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
#[derive(Default)]
struct MapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[repr(C)]
#[derive(Default)]
struct FbCmd2 {
    fb_id: u32,
    width: u32,
    height: u32,
    pixel_format: u32,
    flags: u32,
    handles: [u32; 4],
    pitches: [u32; 4],
    offsets: [u32; 4],
    modifier: [u64; 4],
}

#[repr(C)]
#[derive(Default)]
struct SetCrtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: ModeInfo,
}

#[repr(C)]
struct DirtyCmd {
    fb_id: u32,
    flags: u32,
    color: u32,
    num_clips: u32,
    clips_ptr: u64,
}

#[repr(C)]
struct ClipRect {
    x1: u16,
    y1: u16,
    x2: u16,
    y2: u16,
}

const GETRESOURCES: libc::c_ulong = iowr(0xA0, std::mem::size_of::<CardRes>());
const SETCRTC: libc::c_ulong = iowr(0xA2, std::mem::size_of::<SetCrtc>());
const GETENCODER: libc::c_ulong = iowr(0xA6, std::mem::size_of::<GetEncoder>());
const GETCONNECTOR: libc::c_ulong = iowr(0xA7, std::mem::size_of::<GetConnector>());
const RMFB: libc::c_ulong = iowr(0xAF, 4);
const DIRTYFB: libc::c_ulong = iowr(0xB1, std::mem::size_of::<DirtyCmd>());
const CREATE_DUMB: libc::c_ulong = iowr(0xB2, std::mem::size_of::<CreateDumb>());
const MAP_DUMB: libc::c_ulong = iowr(0xB3, std::mem::size_of::<MapDumb>());
const DESTROY_DUMB: libc::c_ulong = iowr(0xB4, 4);
const ADDFB2: libc::c_ulong = iowr(0xB8, std::mem::size_of::<FbCmd2>());
const SET_MASTER: libc::c_ulong = io(0x1e);
const FORMAT_RGB565: u32 = u32::from_le_bytes(*b"RG16");

fn ioctl<T>(fd: RawFd, req: libc::c_ulong, arg: &mut T) -> io::Result<()> {
    // SAFETY: each request number above is paired with the struct it names.
    let r = unsafe { libc::ioctl(fd, req as _, arg as *mut T) };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// The panel's DRM node: the udev name the M5 image creates, then card1.
pub fn find() -> Option<PathBuf> {
    ["/dev/dri/cardputer-zero-internal", "/dev/dri/card1"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

/// Everything that exists only while we hold the device.
struct Open {
    file: std::fs::File,
    fb_id: u32,
    handle: u32,
    map: *mut u8,
    size: usize,
    pitch: usize,
}

pub struct Drm {
    path: PathBuf,
    width: i32,
    height: i32,
    open: Option<Open>,
    /// What the panel shows, to find the rectangle that changed without
    /// reading the dumb buffer, which may be uncached.
    front: Canvas,
    /// fbdev node handed to apps.
    pub fbdev: String,
    /// While an app runs: fbdev, for the ESC hint and battery banner.
    overlay: Option<crate::fb::Framebuffer>,
}

impl Drm {
    pub fn open(path: &Path, fbdev: String) -> io::Result<Drm> {
        let mut d = Drm {
            path: path.to_path_buf(),
            width: 0,
            height: 0,
            open: None,
            front: Canvas::new(0, 0),
            fbdev,
            overlay: None,
        };
        d.attach()?;
        Ok(d)
    }

    fn attach(&mut self) -> io::Result<()> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)?;
        let fd = file.as_raw_fd();
        // The first opener is master already; this covers a race with fb_load.
        let _ = ioctl(fd, SET_MASTER, &mut ());

        let mut res = CardRes::default();
        ioctl(fd, GETRESOURCES, &mut res)?;
        let mut crtcs = vec![0u32; res.count_crtcs as usize];
        let mut conns = vec![0u32; res.count_connectors as usize];
        let mut res2 = CardRes {
            crtc_id_ptr: crtcs.as_mut_ptr() as u64,
            connector_id_ptr: conns.as_mut_ptr() as u64,
            count_crtcs: res.count_crtcs,
            count_connectors: res.count_connectors,
            ..Default::default()
        };
        ioctl(fd, GETRESOURCES, &mut res2)?;

        let (conn_id, mode, encoder_id) = conns
            .iter()
            .find_map(|&id| {
                let mut c = GetConnector {
                    connector_id: id,
                    ..Default::default()
                };
                ioctl(fd, GETCONNECTOR, &mut c).ok()?;
                let mut modes = vec![ModeInfo::default(); c.count_modes as usize];
                let mut c2 = GetConnector {
                    connector_id: id,
                    modes_ptr: modes.as_mut_ptr() as u64,
                    count_modes: c.count_modes,
                    ..Default::default()
                };
                ioctl(fd, GETCONNECTOR, &mut c2).ok()?;
                // connection 1 is "connected".
                (c2.connection == 1 && !modes.is_empty()).then(|| (id, modes[0], c2.encoder_id))
            })
            .ok_or_else(|| io::Error::other("no connected panel"))?;
        let mut enc = GetEncoder {
            encoder_id,
            ..Default::default()
        };
        let crtc_id = match ioctl(fd, GETENCODER, &mut enc) {
            Ok(()) if enc.crtc_id != 0 => enc.crtc_id,
            _ => (0..crtcs.len())
                .find(|i| enc.possible_crtcs & (1 << i) != 0)
                .map(|i| crtcs[i])
                .or_else(|| crtcs.first().copied())
                .ok_or_else(|| io::Error::other("no crtc"))?,
        };

        let (w, h) = (mode.hdisplay as u32, mode.vdisplay as u32);
        let mut dumb = CreateDumb {
            width: w,
            height: h,
            bpp: 16,
            ..Default::default()
        };
        ioctl(fd, CREATE_DUMB, &mut dumb)?;
        let mut fb = FbCmd2 {
            width: w,
            height: h,
            pixel_format: FORMAT_RGB565,
            ..Default::default()
        };
        fb.handles[0] = dumb.handle;
        fb.pitches[0] = dumb.pitch;
        ioctl(fd, ADDFB2, &mut fb)?;
        let mut map = MapDumb {
            handle: dumb.handle,
            ..Default::default()
        };
        ioctl(fd, MAP_DUMB, &mut map)?;
        let size = dumb.size as usize;
        // SAFETY: mapping the dumb buffer at the offset the kernel gave us.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                map.offset as libc::off_t,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let mut conn = conn_id;
        let mut set = SetCrtc {
            set_connectors_ptr: &mut conn as *mut u32 as u64,
            count_connectors: 1,
            crtc_id,
            fb_id: fb.fb_id,
            mode_valid: 1,
            mode,
            ..Default::default()
        };
        ioctl(fd, SETCRTC, &mut set)?;
        self.width = w as i32;
        self.height = h as i32;
        self.front = Canvas::new(self.width, self.height);
        self.open = Some(Open {
            file,
            fb_id: fb.fb_id,
            handle: dumb.handle,
            map: ptr as *mut u8,
            size,
            pitch: dumb.pitch as usize,
        });
        Ok(())
    }

    fn detach(&mut self) {
        let Some(o) = self.open.take() else { return };
        let fd = o.file.as_raw_fd();
        // SAFETY: unmapping what `attach` mapped.
        unsafe { libc::munmap(o.map as *mut libc::c_void, o.size) };
        let mut fb_id = o.fb_id;
        let _ = ioctl(fd, RMFB, &mut fb_id);
        let mut handle = o.handle;
        let _ = ioctl(fd, DESTROY_DUMB, &mut handle);
        // Dropping the file closes the last DRM fd; the kernel then puts its
        // fbdev picture back on the panel for the app.
    }

    /// Copies `area` of `c` into the dumb buffer and sends it.
    fn send(&mut self, c: &Canvas, area: Rect) {
        let Some(o) = &self.open else { return };
        for y in area.y..area.bottom() {
            let src = &c.row(y)[area.x as usize..area.right() as usize];
            // SAFETY: rows below height lie inside the buffer; pitch covers width.
            let dst = unsafe {
                std::slice::from_raw_parts_mut(
                    o.map.add(y as usize * o.pitch + area.x as usize * 2) as *mut u16,
                    src.len(),
                )
            };
            dst.copy_from_slice(src);
            self.front.copy_span_from(c, y, area.x, area.w);
        }
        let mut clip = ClipRect {
            x1: area.x as u16,
            y1: area.y as u16,
            x2: area.right() as u16,
            y2: area.bottom() as u16,
        };
        let mut cmd = DirtyCmd {
            fb_id: o.fb_id,
            flags: 0,
            color: 0,
            num_clips: 1,
            clips_ptr: &mut clip as *mut ClipRect as u64,
        };
        let _ = ioctl(o.file.as_raw_fd(), DIRTYFB, &mut cmd);
    }
}

impl Drop for Drm {
    fn drop(&mut self) {
        self.detach();
    }
}

/// The smallest rectangle inside `area` where `a` and `b` differ.
pub fn changed_rect(a: &Canvas, b: &Canvas, area: Rect) -> Option<Rect> {
    let area = area.intersect(&a.bounds()).intersect(&b.bounds());
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for y in area.y..area.bottom() {
        let (ra, rb) = (
            &a.row(y)[area.x as usize..area.right() as usize],
            &b.row(y)[area.x as usize..area.right() as usize],
        );
        if ra == rb {
            continue;
        }
        let first = ra.iter().zip(rb).position(|(p, q)| p != q).unwrap_or(0) as i32;
        let last = ra.iter().zip(rb).rposition(|(p, q)| p != q).unwrap_or(0) as i32;
        x0 = x0.min(area.x + first);
        x1 = x1.max(area.x + last + 1);
        y0 = y0.min(y);
        y1 = y1.max(y + 1);
    }
    (y0 != i32::MAX).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

impl crate::display::Display for Drm {
    fn flush(&mut self, c: &Canvas, area: Rect) -> usize {
        if let Some(fb) = &mut self.overlay {
            return fb.flush(c, area);
        }
        match changed_rect(c, &self.front, area) {
            Some(r) => {
                self.send(c, r);
                r.h as usize
            }
            None => 0,
        }
    }

    fn flush_all(&mut self, c: &Canvas) {
        let r = c
            .bounds()
            .intersect(&Rect::new(0, 0, self.width, self.height));
        self.send(c, r);
    }

    fn read_into(&mut self, c: &mut Canvas) {
        match &mut self.overlay {
            Some(fb) => fb.read_into(c),
            None => *c = self.front.clone(),
        }
    }

    fn device(&self) -> String {
        self.fbdev.clone()
    }

    fn suspend(&mut self) {
        self.detach();
        self.overlay = crate::fb::Framebuffer::open(Path::new(&self.fbdev)).ok();
    }

    fn resume(&mut self) {
        self.overlay = None;
        if self.open.is_none() {
            if let Err(e) = self.attach() {
                eprintln!("gh0st: DRM {}: {e}", self.path.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_the_kernel_headers() {
        assert_eq!(std::mem::size_of::<ModeInfo>(), 68);
        assert_eq!(std::mem::size_of::<SetCrtc>(), 104);
        assert_eq!(std::mem::size_of::<FbCmd2>(), 104);
        assert_eq!(GETRESOURCES, 0xC04064A0);
        assert_eq!(DIRTYFB, 0xC01864B1);
        assert_eq!(ADDFB2, 0xC06864B8);
        assert_eq!(FORMAT_RGB565, 0x3631_4752);
    }

    #[test]
    fn the_changed_rect_is_tight() {
        let a = Canvas::new(320, 170);
        let mut b = Canvas::new(320, 170);
        assert_eq!(changed_rect(&a, &b, a.bounds()), None);
        b.fill(Rect::new(80, 20, 70, 60), 0xffff);
        b.fill(Rect::new(100, 100, 5, 5), 0xffff);
        assert_eq!(
            changed_rect(&a, &b, a.bounds()),
            Some(Rect::new(80, 20, 70, 85))
        );
    }
}
