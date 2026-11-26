//! The panel through Linux fbdev (`/dev/fb_lcd`, DRM's fbdev emulation on the
//! current M5 kernel). A flush compares each dirty row of the back buffer
//! with what the mapping holds and writes only the span that differs. The
//! kernel pushes pages that were written, so an unchanged pixel costs no SPI
//! time. Reads from the mapping do not mark pages dirty.

use crate::canvas::Canvas;
use crate::geom::Rect;
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;
/// `_IO('F', 0xC0)` in gh0st's panel driver: returns once the written pages
/// are on the wire. Stock fbdev answers ENOTTY.
const GH0ST_FBIO_PRESENT: libc::c_ulong = 0x46C0;
/// `_IOW('F', 0xC1, struct gh0st_fb_overlay)`: sets or clears an overlay the
/// driver lays over every send. Stock fbdev answers ENOTTY.
const GH0ST_FBIO_OVERLAY: libc::c_ulong = 0x4020_46C1;

/// `struct gh0st_fb_overlay` in the panel driver. w or h 0 clears the slot.
#[repr(C)]
struct OverlayArg {
    slot: u32,
    flags: u32,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    pixels: u64,
    mask: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Bitfield {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct VarInfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: Bitfield,
    green: Bitfield,
    blue: Bitfield,
    transp: Bitfield,
    rest: [u32; 20],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FixInfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    kind: u32,
    type_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    mmio_start: libc::c_ulong,
    mmio_len: u32,
    accel: u32,
    capabilities: u16,
    reserved: [u16; 2],
}

/// The device: `$LV_LINUX_FBDEV_DEVICE`, `/dev/fb_lcd`, an ST7789 entry in
/// `/proc/fb`, then `/dev/fb0`. The same order as stock.
pub fn find() -> PathBuf {
    if let Some(p) = std::env::var_os("LV_LINUX_FBDEV_DEVICE") {
        return p.into();
    }
    if Path::new("/dev/fb_lcd").exists() {
        return "/dev/fb_lcd".into();
    }
    let st7789 = std::fs::read_to_string("/proc/fb").ok().and_then(|s| {
        s.lines()
            .find(|l| l.contains("st7789"))
            .and_then(|l| l.split_whitespace().next()?.parse::<u32>().ok())
    });
    match st7789 {
        Some(n) => format!("/dev/fb{n}").into(),
        None => "/dev/fb0".into(),
    }
}

pub struct Framebuffer {
    file: std::fs::File,
    map: *mut u8,
    len: usize,
    pub width: i32,
    pub height: i32,
    stride: usize,
    bpp: u32,
    var: VarInfo,
    /// The driver supports `GH0ST_FBIO_PRESENT`.
    can_present: bool,
    /// The driver takes overlays: None until first asked.
    can_overlay: Option<bool>,
    pub path: PathBuf,
}

impl Framebuffer {
    pub fn open(path: &Path) -> io::Result<Framebuffer> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        let fd = file.as_raw_fd();
        let mut var = VarInfo::default();
        // SAFETY: the structs match linux/fb.h for this ioctl; fix is
        // zero-initialized plain data.
        let fix = unsafe {
            let mut fix: FixInfo = std::mem::zeroed();
            if libc::ioctl(fd, FBIOGET_VSCREENINFO as _, &mut var) != 0
                || libc::ioctl(fd, FBIOGET_FSCREENINFO as _, &mut fix) != 0
            {
                return Err(io::Error::last_os_error());
            }
            fix
        };
        if var.bits_per_pixel != 16 && var.bits_per_pixel != 32 {
            return Err(io::Error::other(format!(
                "{} bpp is not supported",
                var.bits_per_pixel
            )));
        }
        let stride = fix.line_length as usize;
        let len = (fix.smem_len as usize).max(stride * var.yres as usize);
        // SAFETY: a shared read-write mapping of the whole framebuffer.
        let map = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if map == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: an ioctl without arguments; nothing is pending yet.
        let can_present = unsafe { libc::ioctl(fd, GH0ST_FBIO_PRESENT as _) } == 0;
        Ok(Framebuffer {
            file,
            map: map as *mut u8,
            len,
            width: var.xres as i32,
            height: var.yres as i32,
            stride,
            bpp: var.bits_per_pixel,
            var,
            can_present,
            can_overlay: None,
            path: path.to_path_buf(),
        })
    }

    fn row_mut(&mut self, y: i32) -> &mut [u8] {
        let start = y as usize * self.stride;
        // SAFETY: rows below yres lie inside the mapping (len >= stride * yres).
        unsafe { std::slice::from_raw_parts_mut(self.map.add(start), self.stride) }
    }

    /// Copies the rows `area` touches from `c`, writing only what changed.
    /// Returns how many rows were written, for the debug log.
    pub fn flush(&mut self, c: &Canvas, area: Rect) -> usize {
        let area = area.intersect(&Rect::new(
            0,
            0,
            self.width.min(c.width()),
            self.height.min(c.height()),
        ));
        let mut written = 0;
        let (bpp, var) = (self.bpp, self.var);
        for y in area.y..area.bottom() {
            let src = c.row(y);
            let dst = self.row_mut(y);
            let changed = if bpp == 16 {
                // SAFETY: the row holds at least width u16 pixels and the
                // mapping is page aligned, so u16 alignment holds.
                let dst16 = unsafe {
                    std::slice::from_raw_parts_mut(dst.as_mut_ptr() as *mut u16, src.len())
                };
                copy_changed(src, dst16)
            } else {
                // SAFETY: as above, for u32 pixels.
                let dst32 = unsafe {
                    std::slice::from_raw_parts_mut(dst.as_mut_ptr() as *mut u32, src.len())
                };
                copy_changed_32(src, dst32, &var)
            };
            written += changed as usize;
        }
        if written > 0 {
            self.sync();
        }
        written
    }

    /// Writes every row of `c`, changed or not.
    pub fn flush_all(&mut self, c: &Canvas) {
        let (bpp, var) = (self.bpp, self.var);
        for y in 0..self.height.min(c.height()) {
            let src = c.row(y);
            let dst = self.row_mut(y);
            if bpp == 16 {
                // SAFETY: as in `flush`.
                let dst16 = unsafe {
                    std::slice::from_raw_parts_mut(dst.as_mut_ptr() as *mut u16, src.len())
                };
                dst16.copy_from_slice(src);
            } else {
                // SAFETY: as in `flush`.
                let dst32 = unsafe {
                    std::slice::from_raw_parts_mut(dst.as_mut_ptr() as *mut u32, src.len())
                };
                for (d, &p) in dst32.iter_mut().zip(src) {
                    *d = to_32(p, &var);
                }
            }
        }
        self.sync();
    }

    /// Sends the written pages now. Deferred I/O would wait 50 ms (`HZ / 20`)
    /// before sending; fsync runs that work at once (`fb_deferred_io_fsync`).
    fn sync(&self) {
        // SAFETY: fsync on an fd this struct owns.
        unsafe { libc::fsync(self.file.as_raw_fd()) };
    }

    /// Blocks until everything written is on the wire. False if the driver
    /// can't tell.
    pub fn present(&self) -> bool {
        // SAFETY: an ioctl without arguments on an fd this struct owns.
        self.can_present
            && unsafe { libc::ioctl(self.file.as_raw_fd(), GH0ST_FBIO_PRESENT as _) } == 0
    }

    /// Reads the panel back into `c`, for a screenshot of a running app.
    pub fn read_into(&mut self, c: &mut Canvas) {
        self.read_rect(c, c.bounds());
    }

    /// Reads back only `area`. 16 bpp rows copy straight across.
    pub fn read_rect(&mut self, c: &mut Canvas, area: Rect) {
        let area = area.intersect(&Rect::new(
            0,
            0,
            self.width.min(c.width()),
            self.height.min(c.height()),
        ));
        let (x0, x1) = (area.x as usize, area.right() as usize);
        let (bpp, var) = (self.bpp, self.var);
        for y in area.y..area.bottom() {
            let row = self.row_mut(y);
            let dst = &mut c.row_mut(y)[x0..x1];
            if bpp == 16 {
                for (i, d) in dst.iter_mut().enumerate() {
                    let b = (x0 + i) * 2;
                    *d = u16::from_ne_bytes([row[b], row[b + 1]]);
                }
                continue;
            }
            for (i, d) in dst.iter_mut().enumerate() {
                let b = (x0 + i) * 4;
                let v = u32::from_ne_bytes([row[b], row[b + 1], row[b + 2], row[b + 3]]);
                let ch = |f: Bitfield| {
                    ((v >> f.offset) & ((1 << f.length) - 1)) << (8 - f.length.min(8))
                };
                *d = crate::color::rgb(ch(var.red) as u8, ch(var.green) as u8, ch(var.blue) as u8);
            }
        }
    }

    /// Hands the driver an overlay. False once the driver has said it
    /// can't (M5's driver, or a 32 bpp panel); it is not asked again.
    pub fn overlay_set(&mut self, slot: u8, area: Rect, px: &[u16], mask: Option<&[u8]>) -> bool {
        let n = (area.w.max(0) * area.h.max(0)) as usize;
        if self.bpp != 16 || self.can_overlay == Some(false) || n == 0 || px.len() != n {
            return false;
        }
        if mask.is_some_and(|m| m.len() != n) {
            return false;
        }
        let arg = OverlayArg {
            slot: slot as u32,
            flags: 0,
            x: area.x as u16,
            y: area.y as u16,
            w: area.w as u16,
            h: area.h as u16,
            pixels: px.as_ptr() as u64,
            mask: mask.map_or(0, |m| m.as_ptr() as u64),
        };
        self.overlay_ioctl(&arg)
    }

    /// Takes a slot off. Asked once even before a set, so a shell that
    /// starts after a crash clears what it left.
    pub fn overlay_clear(&mut self, slot: u8) {
        if self.bpp != 16 || self.can_overlay == Some(false) {
            return;
        }
        let arg = OverlayArg {
            slot: slot as u32,
            flags: 0,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            pixels: 0,
            mask: 0,
        };
        self.overlay_ioctl(&arg);
    }

    fn overlay_ioctl(&mut self, arg: &OverlayArg) -> bool {
        // SAFETY: the driver reads the struct and copies w*h pixels and mask
        // bytes from the pointers in it, which outlive the call.
        if unsafe { libc::ioctl(self.file.as_raw_fd(), GH0ST_FBIO_OVERLAY as _, arg) } == 0 {
            self.can_overlay = Some(true);
            return true;
        }
        // Only "no such call" settles it; a refused rect fails this once.
        let e = io::Error::last_os_error().raw_os_error();
        if matches!(e, Some(libc::ENOTTY | libc::EOPNOTSUPP)) {
            self.can_overlay = Some(false);
        }
        false
    }
}

impl Drop for Framebuffer {
    fn drop(&mut self) {
        // SAFETY: unmapping the mapping made in `open`.
        unsafe { libc::munmap(self.map as *mut libc::c_void, self.len) };
    }
}

/// Writes the span of `dst` that differs from `src`, first changed pixel to
/// last. True if anything was written.
pub fn copy_changed(src: &[u16], dst: &mut [u16]) -> bool {
    let n = src.len().min(dst.len());
    let Some(first) = (0..n).find(|&i| src[i] != dst[i]) else {
        return false;
    };
    let last = (first..n)
        .rev()
        .find(|&i| src[i] != dst[i])
        .unwrap_or(first);
    dst[first..=last].copy_from_slice(&src[first..=last]);
    true
}

fn to_32(c: u16, var: &VarInfo) -> u32 {
    let [r, g, b] = crate::color::to_rgb8(c);
    let put = |v: u8, f: Bitfield| ((v as u32) >> (8 - f.length.min(8))) << f.offset;
    put(r, var.red)
        | put(g, var.green)
        | put(b, var.blue)
        | if var.transp.length > 0 {
            ((1 << var.transp.length) - 1) << var.transp.offset
        } else {
            0
        }
}

fn copy_changed_32(src: &[u16], dst: &mut [u32], var: &VarInfo) -> bool {
    let n = src.len().min(dst.len());
    let mut any = false;
    for i in 0..n {
        let v = to_32(src[i], var);
        if dst[i] != v {
            dst[i] = v;
            any = true;
        }
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_changed_span_is_written() {
        let src = [1u16, 2, 9, 4, 9, 6];
        let mut dst = [1u16, 2, 3, 4, 5, 6];
        assert!(copy_changed(&src, &mut dst));
        assert_eq!(dst, src);
        assert!(
            !copy_changed(&src, &mut dst),
            "a second flush writes nothing"
        );
    }

    #[test]
    fn rgb565_widens_to_xrgb8888() {
        let var = VarInfo {
            red: Bitfield {
                offset: 16,
                length: 8,
                msb_right: 0,
            },
            green: Bitfield {
                offset: 8,
                length: 8,
                msb_right: 0,
            },
            blue: Bitfield {
                offset: 0,
                length: 8,
                msb_right: 0,
            },
            ..Default::default()
        };
        assert_eq!(to_32(0xffff, &var), 0x00ff_ffff);
        assert_eq!(to_32(crate::color::rgb(255, 0, 0), &var), 0x00ff_0000);
    }
}
