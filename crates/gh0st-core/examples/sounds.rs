//! Writes every system sound as a WAV into `mockups/sounds`, to listen to
//! off the device: `cargo run -p gh0st-core --example sounds`.

use gh0st_core::sound::{self, Sound};

fn main() -> std::io::Result<()> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mockups/sounds");
    std::fs::create_dir_all(&dir)?;
    for s in Sound::ALL {
        let path = dir.join(format!("{}.wav", s.name()));
        std::fs::write(&path, sound::wav(&s.render()))?;
        println!("{}", path.display());
    }
    Ok(())
}
