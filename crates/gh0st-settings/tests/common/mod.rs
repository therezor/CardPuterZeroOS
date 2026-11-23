//! The system tools Settings asks, as silent stubs first on PATH, so the
//! host's own never answer and every machine runs the same test.

use std::os::unix::fs::PermissionsExt;

/// What Settings runs, each a stub that says nothing and succeeds.
const TOOLS: &[&str] = &[
    "nmcli",
    "bluetoothctl",
    "timedatectl",
    "systemctl",
    "wpctl",
    "vcgencmd",
    "pkexec",
    "iw",
    "rfkill",
    "passwd",
];

pub fn stub_tools() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::temp_dir().join(format!("gh0st-settings-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for t in TOOLS {
            let p = dir.join(t);
            std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = format!("{}:/usr/bin:/bin", dir.display());
        // SAFETY: before any test here starts a page, and nothing else in
        // this process reads the environment meanwhile.
        unsafe { std::env::set_var("PATH", path) };
    });
}
