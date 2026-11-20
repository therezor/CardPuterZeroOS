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
    _file: std::fs::File,
    map: *mut u8,
    len: usize,
    pub width: i32,
    pub height: i32,
    stride: usize,
    bpp: u32,
    var: VarInfo,
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
        Ok(Framebuffer {
            _file: file,
            map: map as *mut u8,
            len,
            width: var.xres as i32,
            height: var.yres as i32,
            stride,
            bpp: var.bits_per_pixel,
            var,
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
    }

    /// Reads the panel back into `c`, for a screenshot of a running app.
    pub fn read_into(&mut self, c: &mut Canvas) {
        let h = self.height.min(c.height());
        for y in 0..h {
            let bpp = self.bpp;
            let var = self.var;
            let row = self.row_mut(y).to_vec();
            for x in 0..self.width.min(c.width()) {
                let px = if bpp == 16 {
                    u16::from_ne_bytes([row[x as usize * 2], row[x as usize * 2 + 1]])
                } else {
                    let i = x as usize * 4;
                    let v = u32::from_ne_bytes([row[i], row[i + 1], row[i + 2], row[i + 3]]);
                    let ch = |b: Bitfield| {
                        ((v >> b.offset) & ((1 << b.length) - 1)) << (8 - b.length.min(8))
                    };
                    crate::color::rgb(ch(var.red) as u8, ch(var.green) as u8, ch(var.blue) as u8)
                };
                c.fill(Rect::new(x, y, 1, 1), px);
            }
        }
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
