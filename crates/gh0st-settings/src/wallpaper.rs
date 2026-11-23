//! The user's own wallpaper: the PNGs they can pick, and the crop that is
//! kept. The crop runs in a child (`gh0st-settings --import-wallpaper`), so
//! a large picture never holds up the page.

use gh0st_app::gfx::theme::CARDPUTER;
use gh0st_app::gfx::wallpaper;
use gh0st_core::config;
use std::path::{Path, PathBuf};

/// Where pictures are picked from.
pub fn pictures_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Pictures"))
}

/// The PNGs in `dir`, by name, sorted. At most 200, so a huge folder can't
/// make a huge page.
pub fn pictures(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.to_ascii_lowercase().ends_with(".png") && !n.starts_with('.'))
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

/// Crops `src` to the screen and keeps it at `dst`, replaced whole.
pub fn import(src: &Path, dst: &Path) -> Result<(), String> {
    let file = std::fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let m = CARDPUTER;
    let pic = wallpaper::import(file, m.width as usize, m.height as usize)?;
    if let Some(dir) = dst.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    config::write_atomic(dst, &wallpaper::encode(&pic)).map_err(|e| e.to_string())
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
        std::fs::write(dir.join(".hidden.png"), "x").unwrap();
        assert_eq!(pictures(&dir), ["A.PNG", "b.png"]);

        let dst = dir.join("conf/wallpaper.png");
        import(&dir.join("b.png"), &dst).unwrap();
        let m = CARDPUTER;
        assert!(wallpaper::load(&dst, m.width as usize, m.height as usize).is_some());
        assert!(import(&dir.join("notes.txt"), &dst).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
