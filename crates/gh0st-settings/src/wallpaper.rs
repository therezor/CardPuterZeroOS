//! The user's own wallpaper: the pictures they can pick, and the crop that
//! is kept. The crop runs in a child (`gh0st-settings --import-wallpaper`),
//! so a large picture never holds up the page. A PNG is read here; any
//! other picture goes through ffmpeg first.

use gh0st_app::gfx::theme::CARDPUTER;
use gh0st_app::gfx::wallpaper;
use gh0st_core::{config, rt};
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
/// ffmpeg's time by the clock, however it spends it: an input that never
/// ends uses no CPU, so its CPU limit never comes.
const FFMPEG_MS: u64 = 60_000;
/// What the page offers: PNG, and what ffmpeg reads on the image.
const TYPES: [&str; 13] = [
    "png", "jpg", "jpeg", "webp", "avif", "heic", "heif", "jxl", "bmp", "gif", "tif", "tiff", "tga",
];

/// Where pictures are picked from.
pub fn pictures_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Pictures"))
}

/// The pictures in `dir`, by name, sorted. At most 200, so a huge folder
/// can't make a huge page.
pub fn pictures(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            let ext = n.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
            !n.starts_with('.') && ext.is_some_and(|e| TYPES.contains(&e.as_str()))
        })
        .collect();
    v.sort();
    v.truncate(200);
    v
}

/// Where the kept crop goes: beside config.json.
pub fn kept() -> PathBuf {
    let path = config::default_path();
    wallpaper::picture_path(path.parent().unwrap_or(Path::new(".")))
}

/// Crops `src` to the screen and keeps it at `dst`, replaced whole. `-` is
/// stdin: the portal hands over the file an app gave it.
pub fn import(src: &Path, dst: &Path) -> Result<(), String> {
    let file = if src == Path::new("-") {
        use std::os::fd::AsFd;
        std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map(File::from)
            .map_err(|e| format!("stdin: {e}"))?
    } else {
        File::open(src).map_err(|e| format!("{}: {e}", src.display()))?
    };
    import_file(file, dst)
}

/// Reads with pread from the start, whatever the file's offset: a file an
/// app handed over shares its offset with that app.
struct FromStart {
    file: File,
    at: u64,
}

impl Read for FromStart {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.file.read_at(buf, self.at)?;
        self.at += n as u64;
        Ok(n)
    }
}

fn import_file(file: File, dst: &Path) -> Result<(), String> {
    let m = CARDPUTER;
    let (w, h) = (m.width as usize, m.height as usize);
    let mut head = [0u8; 8];
    let png = file.read_at(&mut head, 0).is_ok_and(|n| n == 8) && &head == PNG_MAGIC;
    let pic = if png {
        wallpaper::import(FromStart { file, at: 0 }, w, h)?
    } else {
        through_ffmpeg(file, w, h, FFMPEG_MS)?
    };
    if let Some(dir) = dst.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    config::write_atomic(dst, &wallpaper::encode(&pic)).map_err(|e| e.to_string())
}

/// ffmpeg's output, read against a deadline.
struct Deadline<R> {
    out: R,
    until: u64,
}

impl<R: Read + std::os::fd::AsRawFd> Read for Deadline<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if rt::now_ms() >= self.until {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            let mut fds = [rt::pollfd(self.out.as_raw_fd())];
            rt::wait(&mut fds, Some(self.until))?;
            if rt::readable(&fds[0]) {
                return self.out.read(buf);
            }
        }
    }
}

/// Any picture ffmpeg reads, as a PNG twice the screen's size: the crop
/// then averages it down as it does a PNG. `ms` is ffmpeg's time.
fn through_ffmpeg(file: File, w: usize, h: usize, ms: u64) -> Result<wallpaper::Picture, String> {
    let scale = format!(
        "scale=w={}:h={}:force_original_aspect_ratio=increase",
        w * 2,
        h * 2
    );
    // /dev/stdin opens the file again, with its own offset. A hostile
    // picture gets 24 MP (as Media's helper), 30 s of CPU and a minute
    // in all, and no protocol but the file it was given.
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-timelimit", "30"])
        .args(["-protocol_whitelist", "file,pipe"])
        .args([
            "-max_pixels",
            "24000000",
            "-i",
            "/dev/stdin",
            "-frames:v",
            "1",
        ])
        .args(["-vf", &scale, "-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(file)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let until = rt::now_ms() + ms;
    let (pic, ended) = match child.stdout.take() {
        Some(out) => {
            let mut out = Deadline { out, until };
            let pic = wallpaper::import(&mut out, w, h);
            // To its end, which ffmpeg closes as it exits, so the wait
            // below can't hang.
            let ended = pic.is_ok() && std::io::copy(&mut out, &mut std::io::sink()).is_ok();
            (pic, ended)
        }
        None => (Err("ffmpeg: no output".into()), false),
    };
    if !ended {
        let _ = child.kill();
    }
    let ok = child.wait().is_ok_and(|s| s.success());
    match pic {
        Ok(p) if ok => Ok(p),
        _ => Err("not a picture ffmpeg reads".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pngs_are_offered_and_the_crop_lands_whole() {
        let dir = std::env::temp_dir().join(format!("gh0st-pics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pic = wallpaper::Picture {
            w: 400,
            h: 300,
            px: vec![[10.0, 200.0, 90.0]; 400 * 300],
        };
        std::fs::write(dir.join("b.png"), wallpaper::encode(&pic)).unwrap();
        std::fs::write(dir.join("A.PNG"), wallpaper::encode(&pic)).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::write(dir.join("c.jpg"), "x").unwrap();
        std::fs::write(dir.join(".hidden.png"), "x").unwrap();
        assert_eq!(pictures(&dir), ["A.PNG", "b.png", "c.jpg"]);

        let dst = dir.join("conf/wallpaper.png");
        import(&dir.join("b.png"), &dst).unwrap();
        let m = CARDPUTER;
        assert!(wallpaper::load(&dst, m.width as usize, m.height as usize).is_some());
        assert!(import(&dir.join("notes.txt"), &dst).is_err());
        // A handed-over file is read from its start, wherever its offset is.
        let mut f = File::open(dir.join("b.png")).unwrap();
        f.read_exact(&mut [0u8; 5]).unwrap();
        std::fs::remove_file(&dst).unwrap();
        import_file(f, &dst).unwrap();
        assert!(dst.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ffmpeg_reads_a_jpeg_and_the_clock_stops_an_input_that_never_ends() {
        // A host without ffmpeg can't run this.
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("gh0st-pics-ff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jpg = dir.join("a.jpg");
        let made = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=640x480",
            ])
            .args(["-frames:v", "1"])
            .arg(&jpg)
            .status()
            .unwrap();
        assert!(made.success());
        let m = CARDPUTER;
        let (w, h) = (m.width as usize, m.height as usize);
        let pic = through_ffmpeg(File::open(&jpg).unwrap(), w, h, 20_000).unwrap();
        assert_eq!((pic.w, pic.h), (w, h));

        // A pipe whose writer never writes nor closes.
        let mut sleeper = Command::new("sleep")
            .arg("30")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pipe = std::os::fd::OwnedFd::from(sleeper.stdout.take().unwrap());
        let t0 = rt::now_ms();
        assert!(through_ffmpeg(File::from(pipe), w, h, 500).is_err());
        assert!(rt::now_ms() - t0 < 5_000, "stopped at its time");
        let _ = sleeper.kill();
        let _ = sleeper.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
