//! A file on a server opens from a copy in the cache, in its app. Its own
//! test binary: it names the apps through the environment.

use gh0st_app::secret::Keyring;
use gh0st_app::testing::Driver;
use gh0st_files::app::{Files, Options};
use gh0st_files::engine::Request;
use gh0st_files::fake::Fake;
use gh0st_files::model::{Entry, Loc};
use gh0st_files::servers::{self, Proto, Server};

#[test]
fn a_server_file_is_copied_then_opened_in_its_app() {
    let dir = std::env::temp_dir().join(format!("gh0st-files-remote-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let apps = dir.join("apps");
    std::fs::create_dir_all(&apps).unwrap();
    let exe = dir.join("text");
    std::fs::write(&exe, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&exe, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::write(
        apps.join("text.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Text\nExec={} %f\nMimeType=text/plain;\n",
            exe.display()
        ),
    )
    .unwrap();
    // SAFETY: this binary runs this one test, before any thread reads it.
    unsafe {
        std::env::set_var("GH0ST_APPS_OURS", &apps);
        std::env::set_var("GH0ST_APPS_STOCK", dir.join("none"));
        std::env::set_var("HOME", &home);
    }
    let mut nas = Server::new(Proto::Sftp);
    nas.name = "nas".into();
    nas.host = "nas.local".into();
    nas.user = "me".into();
    nas.path = "/".into();
    servers::save(&dir.join("data/servers"), &[nas]).unwrap();
    let f = Fake::new().folder("nas", "/", vec![Entry::file("notes.txt", 3)]);
    let log = f.log.clone();
    let mut d = Driver::new(Files::with(Options {
        home: home.clone(),
        data: Some(dir.join("data")),
        engine: Box::new(f),
        keyring: Keyring::over(None),
        pick: None,
    }));
    d.frame();
    // Places on side B: Home, Card, Trash, nas. Then the file.
    for k in ["tab", "x", "x", "x", "enter", "x", "enter"] {
        d.keys(k);
        d.frame();
    }
    let copied: Vec<(Vec<Loc>, Loc)> = log
        .borrow()
        .iter()
        .filter_map(|r| match r {
            Request::Copy { items, to, .. } => Some((items.clone(), to.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(copied.len(), 1, "{:?}", log.borrow());
    let Loc::Local(cache) = &copied[0].1 else {
        panic!("into the cache")
    };
    assert!(cache.starts_with(home.join(".cache/gh0st-files/open")));
    let opened = d.opened_requests();
    assert_eq!(opened, [(None, cache.join("notes.txt"))]);
    let _ = std::fs::remove_dir_all(&dir);
}
