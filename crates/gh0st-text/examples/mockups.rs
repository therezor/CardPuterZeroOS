//! Text's real page over fixture files, into mockups/text. Both key models
//! are drawn, so they can be compared. Run from the workspace root:
//!
//!     cargo run -p gh0st-text --example mockups

use gh0st_app::testing::Driver;
use gh0st_text::app::{Editor, KeyModel, Options};
use std::path::{Path, PathBuf};

const RUST: &str = r#"//! A tiny cache for decoded icons.

use std::collections::HashMap;

/// Icons by name, decoded once.
pub struct Cache<'a> {
    map: HashMap<&'a str, Vec<u16>>,
    hits: u32,
}

impl<'a> Cache<'a> {
    pub fn new() -> Self {
        Cache { map: HashMap::new(), hits: 0 }
    }

    /// The icon, decoding it the first time.
    pub fn get(&mut self, name: &'a str) -> &[u16] {
        if self.map.contains_key(name) {
            self.hits += 1;
        }
        self.map.entry(name).or_insert_with(|| decode(name, 0x1F))
    }
}

fn decode(name: &str, mask: u16) -> Vec<u16> {
    println!("decoding {name}");
    vec![mask; 16 * 16]
}
"#;

const MARKDOWN: &str = "# Gh0st OS\n\nA lean OS for the **CardputerZero**. Read the \
[guide](docs/sdk.md) first.\n\n## Build\n\n```sh\ncargo build --release\n```\n\n- \
Settings\n- The `terminal`\n- Files\n\n> Nothing here is measured yet.\n";

struct Scene {
    name: &'static str,
    file: &'static str,
    keys: KeyModel,
    size: usize,
    script: &'static str,
    restore: Option<usize>,
    index_budget: Option<u64>,
}

const fn scene(
    name: &'static str,
    file: &'static str,
    keys: KeyModel,
    script: &'static str,
) -> Scene {
    Scene {
        name,
        file,
        keys,
        size: 0,
        script,
        restore: None,
        index_budget: None,
    }
}

const A: KeyModel = KeyModel::Modeless;
const B: KeyModel = KeyModel::Modal;

/// Down to line 18, on `contains_key`.
const TO_LINE: &str = "fn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c\nfn+c";

fn scenes() -> Vec<Scene> {
    vec![
        scene("a01-code", "cache.rs", A, TO_LINE),
        scene(
            "a02-select",
            "cache.rs",
            A,
            "fn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nshift+fn+x\nshift+fn+x\nshift+fn+c\nshift+fn+c",
        ),
        scene(
            "a03-typed",
            "cache.rs",
            A,
            "fn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nenter\ntype // TODO: evict",
        ),
        scene("a04-find", "cache.rs", A, "ctrl+f\ntype self\nenter"),
        scene("a05-goto", "cache.rs", A, "ctrl+g\ntype 21"),
        scene("a06-help", "cache.rs", A, "help"),
        scene("a07-quit", "cache.rs", A, "type x\nesc"),
        scene("b01-view", "cache.rs", B, TO_LINE),
        scene(
            "b02-edit",
            "cache.rs",
            B,
            "fn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nfn+x\nenter\nfn+n\ntype  // per name",
        ),
        scene("b03-menu", "cache.rs", B, "8"),
        scene("b04-help", "cache.rs", B, "help"),
        scene("b05-find", "cache.rs", B, "/\ntype decode\nenter"),
        Scene {
            size: 2,
            ..scene("size-8x16", "cache.rs", A, TO_LINE)
        },
        scene("size-6x12", "cache.rs", A, TO_LINE),
        scene("markdown", "README.md", A, ""),
        scene(
            "replace",
            "cache.rs",
            A,
            "ctrl+f\ntype hits\nenter\nesc\nctrl+r",
        ),
        scene("hex", "icon.bin", A, "fn+x\nfn+x"),
        Scene {
            restore: Some(3),
            ..scene("restore", "cache.rs", A, "")
        },
        Scene {
            index_budget: Some(256 << 10),
            ..scene("log-indexing", "service.log", A, "fn+m\nfn+m")
        },
    ]
}

fn fixtures(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("cache.rs"), RUST).unwrap();
    std::fs::write(dir.join("README.md"), MARKDOWN).unwrap();
    let bin: Vec<u8> = (0..512u32).map(|i| (i * 37 % 256) as u8).collect();
    std::fs::write(dir.join("icon.bin"), bin).unwrap();
    let mut log = String::new();
    let mut i = 0u32;
    while log.len() < 4 << 20 {
        let lvl = ["INFO ", "DEBUG", "WARN ", "ERROR"][(i % 7 % 4) as usize];
        log.push_str(&format!(
            "2026-11-24T12:{:02}:{:02}Z {lvl} gh0st-store[812]: line {i} fetched catalog in {} ms\n",
            i / 60 % 60,
            i % 60,
            i % 250
        ));
        i += 1;
    }
    std::fs::write(dir.join("service.log"), log).unwrap();
}

fn each_scene(mut each: impl FnMut(&str, &mut Driver)) {
    let dir = std::env::temp_dir().join(format!("gh0st-text-mockups-{}", std::process::id()));
    fixtures(&dir);
    for s in scenes() {
        let opts = Options {
            keys: Some(s.keys),
            size: Some(s.size),
            index_budget: s.index_budget,
            restore: s.restore,
        };
        let path = dir.join(s.file);
        let mut d = Driver::new(Editor::open(Some(&path), opts));
        if s.index_budget.is_none() {
            d.wait(50);
        }
        d.keys(s.script);
        each(s.name, &mut d);
    }
}

fn save(d: &mut Driver, out: &Path, name: &str) {
    d.save(&out.join(format!("{name}.png"))).unwrap();
}

#[cfg_attr(test, allow(dead_code))]
fn main() {
    let out = PathBuf::from("mockups/text");
    std::fs::create_dir_all(&out).unwrap();
    each_scene(|name, d| save(d, &out, name));
    println!("wrote {}", out.display());
}

/// The mockups are the fixtures: every scene, in every palette, under the
/// design lint.
#[test]
fn every_scene_keeps_the_design_rules() {
    let mut found = Vec::new();
    each_scene(|name, d| {
        for p in gh0st_app::gfx::theme::PALETTES {
            d.palette(p.name);
            found.extend(
                d.lint()
                    .into_iter()
                    .map(|f| format!("{name} {}: {f}", p.name)),
            );
        }
    });
    assert!(found.is_empty(), "{found:#?}");
}
