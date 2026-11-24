//! The USB keyboard: a configfs gadget with three HID functions (a boot
//! keyboard, a boot mouse, consumer control) on the board's USB controller.
//! It works while the side switch is on Slave, through the right-hand
//! USB-C port. Root builds it: `gh0st-helper usb-hid hold`.
//!
//! The filesystem steps go through [`Fs`], so tests check the tree and the
//! teardown order without configfs.

use std::io;
use std::path::{Path, PathBuf};

use crate::hid;

pub const CONFIGFS: &str = "/sys/kernel/config/usb_gadget";
pub const GADGET: &str = "gh0st-keyboard";
pub const UDC_CLASS: &str = "/sys/class/udc";
pub const HIDG_CLASS: &str = "/sys/class/hidg";

/// The Linux Foundation's IDs for a composite gadget, as configfs guides
/// use. Nothing here names the device or the person: the serial is fixed.
const VENDOR: &str = "0x1d6b";
const PRODUCT: &str = "0x0104";
const MANUFACTURER: &str = "Gh0st";
const PRODUCT_NAME: &str = "Gh0st Keyboard";
const SERIAL: &str = "gh0st";

pub struct Function {
    /// `hid.<name>` in configfs, and the name the helper prints.
    pub name: &'static str,
    pub subclass: u8,
    pub protocol: u8,
    pub len: u16,
    pub desc: &'static [u8],
}

pub const FUNCTIONS: [Function; 3] = [
    Function {
        name: "keyboard",
        subclass: 1,
        protocol: 1,
        len: 8,
        desc: hid::USB_KEYBOARD,
    },
    Function {
        name: "mouse",
        subclass: 1,
        protocol: 2,
        len: 5,
        desc: hid::USB_MOUSE,
    },
    Function {
        name: "consumer",
        subclass: 0,
        protocol: 0,
        len: 2,
        desc: hid::USB_CONSUMER,
    },
];

/// The filesystem steps a gadget takes.
pub trait Fs {
    fn mkdir(&mut self, p: &Path) -> io::Result<()>;
    fn write(&mut self, p: &Path, data: &[u8]) -> io::Result<()>;
    fn symlink(&mut self, target: &Path, link: &Path) -> io::Result<()>;
    fn unlink(&mut self, p: &Path) -> io::Result<()>;
    fn rmdir(&mut self, p: &Path) -> io::Result<()>;
}

/// The real filesystem.
pub struct Real;

impl Fs for Real {
    fn mkdir(&mut self, p: &Path) -> io::Result<()> {
        match std::fs::create_dir(p) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            r => r,
        }
    }
    fn write(&mut self, p: &Path, data: &[u8]) -> io::Result<()> {
        std::fs::write(p, data)
    }
    fn symlink(&mut self, target: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }
    fn unlink(&mut self, p: &Path) -> io::Result<()> {
        std::fs::remove_file(p)
    }
    fn rmdir(&mut self, p: &Path) -> io::Result<()> {
        std::fs::remove_dir(p)
    }
}

pub fn root() -> PathBuf {
    Path::new(CONFIGFS).join(GADGET)
}

/// Makes the gadget under `root`, unbound.
pub fn build(fs: &mut dyn Fs, root: &Path) -> io::Result<()> {
    fs.mkdir(root)?;
    fs.write(&root.join("idVendor"), VENDOR.as_bytes())?;
    fs.write(&root.join("idProduct"), PRODUCT.as_bytes())?;
    fs.write(&root.join("bcdDevice"), b"0x0100")?;
    fs.write(&root.join("bcdUSB"), b"0x0200")?;
    let strings = root.join("strings/0x409");
    fs.mkdir(&root.join("strings"))?;
    fs.mkdir(&strings)?;
    fs.write(&strings.join("manufacturer"), MANUFACTURER.as_bytes())?;
    fs.write(&strings.join("product"), PRODUCT_NAME.as_bytes())?;
    fs.write(&strings.join("serialnumber"), SERIAL.as_bytes())?;
    let config = root.join("configs/c.1");
    fs.mkdir(&root.join("configs"))?;
    fs.mkdir(&config)?;
    // Self-powered: it has its own battery, so it asks the host for little.
    fs.write(&config.join("bmAttributes"), b"0xc0")?;
    fs.write(&config.join("MaxPower"), b"100")?;
    fs.mkdir(&config.join("strings"))?;
    fs.mkdir(&config.join("strings/0x409"))?;
    fs.write(&config.join("strings/0x409/configuration"), b"Keyboard")?;
    fs.mkdir(&root.join("functions"))?;
    for f in &FUNCTIONS {
        let dir = root.join("functions").join(format!("hid.{}", f.name));
        fs.mkdir(&dir)?;
        fs.write(&dir.join("subclass"), f.subclass.to_string().as_bytes())?;
        fs.write(&dir.join("protocol"), f.protocol.to_string().as_bytes())?;
        fs.write(&dir.join("report_length"), f.len.to_string().as_bytes())?;
        fs.write(&dir.join("report_desc"), f.desc)?;
        fs.symlink(&dir, &config.join(format!("hid.{}", f.name)))?;
    }
    Ok(())
}

/// Binds the gadget to `udc`: the host sees a keyboard plugged in.
pub fn bind(fs: &mut dyn Fs, root: &Path, udc: &str) -> io::Result<()> {
    fs.write(&root.join("UDC"), udc.as_bytes())
}

/// Takes the gadget down, in the order configfs needs: unbound, links,
/// then each directory from the leaves up. Steps that fail are skipped,
/// so a half-built gadget goes too.
pub fn teardown(fs: &mut dyn Fs, root: &Path) {
    let _ = fs.write(&root.join("UDC"), b"\n");
    let config = root.join("configs/c.1");
    for f in &FUNCTIONS {
        let _ = fs.unlink(&config.join(format!("hid.{}", f.name)));
    }
    let _ = fs.rmdir(&config.join("strings/0x409"));
    let _ = fs.rmdir(&config);
    for f in &FUNCTIONS {
        let _ = fs.rmdir(&root.join("functions").join(format!("hid.{}", f.name)));
    }
    let _ = fs.rmdir(&root.join("strings/0x409"));
    let _ = fs.rmdir(root);
}

/// The board's USB device controller, if it has one.
pub fn udc(class: &Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(class)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names.into_iter().next()
}

/// Each function's device node, by the `dev` numbers configfs shows and
/// `hidg_class` lists.
pub fn nodes(root: &Path, hidg_class: &Path) -> Vec<(&'static str, PathBuf)> {
    let dev = |p: &Path| {
        std::fs::read_to_string(p)
            .ok()
            .map(|s| s.trim().to_string())
    };
    let class: Vec<(String, String)> = std::fs::read_dir(hidg_class)
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    let n = dev(&e.path().join("dev"))?;
                    Some((n, e.file_name().to_string_lossy().into_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    FUNCTIONS
        .iter()
        .filter_map(|f| {
            let want = dev(&root
                .join("functions")
                .join(format!("hid.{}", f.name))
                .join("dev"))?;
            let (_, name) = class.iter().find(|(n, _)| *n == want)?;
            Some((f.name, Path::new("/dev").join(name)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every step, as text, for the order; writes also land in a map.
    #[derive(Default)]
    struct Log(Vec<String>);

    impl Fs for Log {
        fn mkdir(&mut self, p: &Path) -> io::Result<()> {
            self.0.push(format!("mkdir {}", p.display()));
            Ok(())
        }
        fn write(&mut self, p: &Path, data: &[u8]) -> io::Result<()> {
            self.0.push(format!("write {} {}", p.display(), data.len()));
            Ok(())
        }
        fn symlink(&mut self, target: &Path, link: &Path) -> io::Result<()> {
            self.0
                .push(format!("ln {} {}", target.display(), link.display()));
            Ok(())
        }
        fn unlink(&mut self, p: &Path) -> io::Result<()> {
            self.0.push(format!("rm {}", p.display()));
            Ok(())
        }
        fn rmdir(&mut self, p: &Path) -> io::Result<()> {
            self.0.push(format!("rmdir {}", p.display()));
            Ok(())
        }
    }

    #[test]
    fn a_built_gadget_has_three_functions_linked() {
        let mut log = Log::default();
        build(&mut log, Path::new("/g")).unwrap();
        let links: Vec<&String> = log.0.iter().filter(|l| l.starts_with("ln ")).collect();
        assert_eq!(links.len(), 3);
        assert!(links[0].ends_with("/g/configs/c.1/hid.keyboard"));
        let desc = format!(
            "write /g/functions/hid.keyboard/report_desc {}",
            hid::USB_KEYBOARD.len()
        );
        assert!(log.0.contains(&desc));
        assert!(
            !log.0.iter().any(|l| l.contains("/UDC")),
            "built, not bound"
        );
    }

    #[test]
    fn teardown_unbinds_first_and_removes_leaves_first() {
        let mut log = Log::default();
        teardown(&mut log, Path::new("/g"));
        assert_eq!(log.0[0], "write /g/UDC 1");
        let pos = |s: &str| log.0.iter().position(|l| l == s).unwrap();
        assert!(pos("rm /g/configs/c.1/hid.mouse") < pos("rmdir /g/configs/c.1"));
        assert!(pos("rmdir /g/configs/c.1") < pos("rmdir /g/functions/hid.mouse"));
        assert_eq!(log.0.last().map(String::as_str), Some("rmdir /g"));
    }

    #[test]
    fn real_build_writes_the_files() {
        let dir = std::env::temp_dir().join(format!("gh0st-hidg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.join(GADGET);
        build(&mut Real, &root).unwrap();
        let read = |p: &str| std::fs::read(root.join(p)).unwrap();
        assert_eq!(read("idProduct"), b"0x0104");
        assert_eq!(read("functions/hid.mouse/protocol"), b"2");
        assert_eq!(
            read("functions/hid.consumer/report_desc"),
            hid::USB_CONSUMER
        );
        // The function nodes, found by their numbers.
        let class = dir.join("hidg");
        for (i, f) in ["keyboard", "mouse", "consumer"].iter().enumerate() {
            std::fs::write(
                root.join(format!("functions/hid.{f}/dev")),
                format!("243:{i}\n"),
            )
            .unwrap();
            std::fs::create_dir_all(class.join(format!("hidg{i}"))).unwrap();
            std::fs::write(class.join(format!("hidg{i}/dev")), format!("243:{i}\n")).unwrap();
        }
        let n = nodes(&root, &class);
        assert_eq!(n[0], ("keyboard", PathBuf::from("/dev/hidg0")));
        assert_eq!(n.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
