//! The Files page over a fixture home folder and a fake worker, into
//! mockups/files. Run from the workspace root:
//!
//!     cargo run -p gh0st-files --example mockups

use gh0st_app::secret::Keyring;
use gh0st_app::testing::Driver;
use gh0st_files::app::{Files, Options};
use gh0st_files::engine::Ask;
use gh0st_files::fake::{CopyScript, Fake, arc_dir, arc_file, at};
use gh0st_files::model::Entry;
use gh0st_files::servers::{self, Proto, Server};
use std::path::{Path, PathBuf};

const T: i64 = 1_795_000_000;

/// A home folder with something in it: files are sparse, so sizes cost
/// nothing.
fn home(base: &Path) -> PathBuf {
    let h = base.join("home");
    let _ = std::fs::remove_dir_all(&h);
    let file = |p: &str, size: u64| {
        let path = h.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(size).unwrap();
    };
    for d in ["Documents", "Music/Albums", "Pictures", "projects"] {
        std::fs::create_dir_all(h.join(d)).unwrap();
    }
    file("Music/intro theme.mp3", 3_412_000);
    file("Music/night drive.flac", 28_140_000);
    file("Music/rain loop.wav", 12_400_000);
    file("Music/track 2.flac", 21_000_000);
    file("Music/track 10.flac", 19_700_000);
    file("Music/lyrics.txt", 1_240);
    file("Downloads/backup.zip", 48_200_000);
    file("Downloads/photos.tar.gz", 112_000_000);
    file("Downloads/firmware.bin", 2_097_152);
    file("Documents/budget.csv", 8_200);
    file("Documents/plan.md", 3_100);
    file("Pictures/beach.png", 1_830_000);
    file(".bashrc", 3_700);
    file("todo.txt", 214);
    std::fs::write(
        h.join("readme.md"),
        "# Field kit\n\nThis card holds the notes, tracks and backups for the trip.\n\n\
         - Music: the playlist for the drive\n- Downloads: firmware and old backups\n\
         - Documents: the plan and the budget\n\nCopy new photos to the NAS each night. \
         The NAS keeps them under backup/photos, one folder a day.\n\nIf the NAS is down, \
         the WebDAV share at work takes them instead.\n",
    )
    .unwrap();
    let trash = h.join(".local/share/Trash/files");
    std::fs::create_dir_all(&trash).unwrap();
    std::fs::write(trash.join("old notes.txt"), b"old").unwrap();
    h
}

fn saved() -> Vec<Server> {
    let mk = |name: &str, proto: Proto, host: &str, user: &str, path: &str| Server {
        name: name.into(),
        host: host.into(),
        user: user.into(),
        path: path.into(),
        ..Server::new(proto)
    };
    vec![
        mk("nas", Proto::Sftp, "nas.local", "me", "/home/me"),
        mk(
            "work",
            Proto::Davs,
            "dav.example.com",
            "me",
            "/remote.php/dav/files/me",
        ),
        mk("printer", Proto::Ftp, "192.168.1.40", "", "/"),
        mk("pi", Proto::Sftp, "raspberrypi.local", "pi", "/home/pi"),
    ]
}

fn fake(copy: CopyScript) -> Fake {
    let mut f = Fake::new()
        .folder(
            "nas",
            "/home/me",
            vec![
                Entry::dir("backup"),
                Entry::dir("music"),
                Entry::dir("photos"),
                at(Entry::file("notes-2025.txt", 4_800), T),
                at(Entry::file("old.zip", 230_000_000), T),
                at(Entry::file("router.conf", 2_100), T),
            ],
        )
        .folder(
            "printer",
            "/",
            vec![Entry::dir("scans"), Entry::file("status.txt", 120)],
        );
    f.archives.insert(
        "backup.zip".into(),
        vec![
            arc_dir("backup/"),
            arc_dir("backup/config/"),
            arc_file("backup/config/wifi.conf", 380),
            arc_file("backup/notes.txt", 12_000),
            arc_file("backup/contacts.vcf", 44_000),
            arc_file("backup/photos/1.png", 2_100_000),
            arc_file("backup/photos/2.png", 1_900_000),
        ],
    );
    f.fail.insert(
        "work".into(),
        "dav.example.com didn't answer in 15 s. Check the address and the network.".into(),
    );
    f.first.insert(
        "pi".into(),
        vec![
            Ask::HostKey {
                server: "pi".into(),
                host: "raspberrypi.local".into(),
                fingerprint: "SHA256:eV9BNqsFgfxmMG5BOGOvU5/SwcJWH8oLEv04xC3YR6s".into(),
            },
            Ask::Password {
                server: "pi".into(),
                retry: false,
            },
        ],
    );
    f.copy = copy;
    f
}

struct Scene {
    name: &'static str,
    copy: CopyScript,
    keys: &'static str,
}

fn scene(name: &'static str, keys: &'static str) -> Scene {
    Scene {
        name,
        copy: CopyScript::default(),
        keys,
    }
}

#[cfg_attr(test, allow(dead_code))]
fn main() {
    let out = PathBuf::from("mockups/files");
    std::fs::create_dir_all(&out).unwrap();
    each_scene(|name, d| {
        d.save(&out.join(format!("{name}.png"))).unwrap();
    });
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

fn each_scene(mut each: impl FnMut(&str, &mut Driver)) {
    let base = std::env::temp_dir().join(format!("gh0st-files-mockups-{}", std::process::id()));
    let copying = CopyScript {
        progress: Some((11_200_000, 49_552_000, "night drive.flac".into())),
        conflict: None,
    };
    let conflict = CopyScript {
        progress: None,
        conflict: Some("intro theme.mp3".into()),
    };
    let scenes = [
        scene("01-home", ""),
        scene("02-places", "tab"),
        scene("03-marked", "x\nx\nenter\nx\nx\nx\nspace\nspace"),
        scene("04-remote", "tab\nx\nx\nx\nx\nenter"),
        scene("05-find", "n\ntype mu"),
        scene(
            "06-copy-ask",
            concat!(
                "tab\nx\nx\nx\nx\nenter\ntab\nx\nx\nenter\nx\nx\nx\nspace\nspace",
                "\n6"
            ),
        ),
        Scene {
            name: "07-copying",
            copy: copying.clone(),
            keys: concat!(
                "tab\nx\nx\nx\nx\nenter\ntab\nx\nx\nenter\nx\nx\nx\nspace\nspace",
                "\n6\nenter"
            ),
        },
        Scene {
            name: "08-transfers",
            copy: copying,
            keys: concat!(
                "tab\nx\nx\nx\nx\nenter\ntab\nx\nx\nenter\nx\nx\nx\nspace\nspace",
                "\n6\nenter\nt"
            ),
        },
        Scene {
            name: "09-conflict",
            copy: conflict,
            keys: concat!(
                "tab\nx\nx\nx\nx\nenter\ntab\nx\nx\nenter\nx\nx\nx\nspace\nspace",
                "\n6\nenter"
            ),
        },
        scene("10-delete-trash", "x\nx\nenter\nx\nx\nx\ndelete"),
        scene(
            "11-delete-remote",
            "tab\nx\nx\nx\nx\nenter\nx\nx\nx\ndelete",
        ),
        scene("12-archive", "x\nenter\nenter\nenter"),
        scene(
            "13-archive-marked",
            "x\nenter\nenter\nenter\nx\nx\nspace\nspace",
        ),
        scene("14-add-server", "tab\nx\nx\nx\nx\nx\nx\nx\nx\nenter"),
        scene(
            "15-server-filled",
            "tab\nx\nx\nx\nx\nx\nx\nx\nx\nenter\nx\nenter\ntype 192.168.1.20:2222\nenter\nx\nenter\ntype me\nenter",
        ),
        scene("16-host-key", "tab\nx\nx\nx\nx\nx\nx\nx\nenter"),
        scene("17-password", "tab\nx\nx\nx\nx\nx\nx\nx\nenter\nenter"),
        scene("18-viewer", "x\nx\nx\nx\nx\nenter"),
        scene(
            "19-viewer-scrolled",
            "x\nx\nx\nx\nx\nenter\nx\nx\nx\nx\nx\nx",
        ),
        scene("20-info", "x\nx\nx\nx\nx\nx\ni"),
        scene("21-empty", "x\nx\nx\nx\nenter"),
        scene("22-error", "tab\nx\nx\nx\nx\nx\nenter"),
        scene("23-plain-ftp", "tab\nx\nx\nx\nx\nx\nx\nenter"),
        scene("24-forget", "tab\nx\nx\nx\nx\n7"),
        scene("25-menu", "8"),
        scene("26-mark-all", "x\nx\nenter\na"),
        scene("27-rename", "x\nx\nenter\nx\nx\nr"),
        scene("28-places-a", "esc"),
        scene("29-help", "help"),
        scene("30-help-down", "help\nx\nx\nx\nx\nx\nx"),
        scene("31-help-more", "help\nx\nx\nx\nx\nx\nx\nx\nx\nx\nx\nx\nx"),
        scene("32-menu-end", "8\nf"),
        scene("33-help-places", "tab\nhelp"),
    ];
    for s in scenes {
        let dir = base.join(s.name);
        let h = home(&dir);
        let data = dir.join("data");
        servers::save(&data.join("servers"), &saved()).unwrap();
        let page = Files::with(Options {
            home: h,
            data: Some(data),
            engine: Box::new(fake(s.copy)),
            keyring: Keyring::over(None),
        });
        let mut d = Driver::new(page);
        d.frame();
        // A frame after each key, as on the device: a list learns its rows
        // when drawn.
        for line in s.keys.lines() {
            d.keys(line);
            d.frame();
        }
        each(s.name, &mut d);
    }
    let _ = std::fs::remove_dir_all(&base);
}
