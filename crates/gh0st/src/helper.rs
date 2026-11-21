//! `gh0st-helper`: the few things Settings needs root for. pkexec runs it as
//! root for the device user, allowed by our polkit rule, so it takes a fixed
//! set of verbs, checks every argument, and never runs a shell.
//!
//!     gh0st-helper logs off|ram|persistent
//!     gh0st-helper ssh on|off
//!     gh0st-helper rfkill bluetooth|wlan on|off
//!     gh0st-helper power off|reboot

use gh0st_core::rfkill::{self, Radio};
use std::process::Command;

/// Sorts after stock's `persist.conf`, so its Storage= wins.
const LOGGING_CONF: &str = "/etc/systemd/journald.conf.d/zz-gh0st-logging.conf";

#[derive(Debug, PartialEq, Eq)]
pub enum Verb {
    Logs(&'static str),
    Ssh(bool),
    Rfkill(Radio, bool),
    Power(bool),
}

pub fn parse(args: &[String]) -> Option<Verb> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let on = |s: &str| match s {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    };
    Some(match a.as_slice() {
        ["logs", "off"] => Verb::Logs("off"),
        ["logs", "ram"] => Verb::Logs("ram"),
        ["logs", "persistent"] => Verb::Logs("persistent"),
        ["ssh", s] => Verb::Ssh(on(s)?),
        ["rfkill", "bluetooth", s] => Verb::Rfkill(Radio::Bluetooth, on(s)?),
        ["rfkill", "wlan", s] => Verb::Rfkill(Radio::Wlan, on(s)?),
        ["power", "off"] => Verb::Power(false),
        ["power", "reboot"] => Verb::Power(true),
        _ => return None,
    })
}

/// The journald drop-in for each logging level.
pub fn logging_conf(level: &str) -> &'static str {
    match level {
        "ram" => "[Journal]\nStorage=volatile\nRuntimeMaxUse=8M\n",
        "persistent" => "[Journal]\nStorage=persistent\nSystemMaxUse=16M\n",
        _ => "[Journal]\nStorage=none\n",
    }
}

fn run(argv: &[&str]) -> Result<(), String> {
    let status = Command::new(argv[0])
        .args(&argv[1..])
        .status()
        .map_err(|e| format!("{}: {e}", argv[0]))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} failed", argv.join(" ")))
    }
}

fn act(v: Verb) -> Result<(), String> {
    match v {
        Verb::Logs(level) => {
            if level == "off" {
                // What is on disk goes too, not only what comes next.
                let _ = run(&["journalctl", "--rotate"]);
                let _ = run(&["journalctl", "--vacuum-time=1s"]);
            }
            std::fs::create_dir_all("/etc/systemd/journald.conf.d").map_err(|e| e.to_string())?;
            gh0st_core::config::write_atomic(
                std::path::Path::new(LOGGING_CONF),
                logging_conf(level).as_bytes(),
            )
            .map_err(|e| format!("{LOGGING_CONF}: {e}"))?;
            run(&["systemctl", "restart", "systemd-journald"])
        }
        Verb::Ssh(on) => {
            let verb = if on { "enable" } else { "disable" };
            run(&["systemctl", verb, "--now", "ssh"])
        }
        Verb::Rfkill(radio, on) => rfkill::set(radio, !on).map_err(|e| format!("/dev/rfkill: {e}")),
        Verb::Power(reboot) => run(&["systemctl", if reboot { "reboot" } else { "poweroff" }]),
    }
}

pub fn main(args: &[String]) -> i32 {
    let Some(v) = parse(args) else {
        eprintln!(
            "usage: gh0st-helper logs off|ram|persistent | ssh on|off | rfkill bluetooth|wlan on|off | power off|reboot"
        );
        return 2;
    };
    match act(v) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("gh0st-helper: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn only_the_fixed_verbs_get_through() {
        assert_eq!(parse(&args("logs ram")), Some(Verb::Logs("ram")));
        assert_eq!(parse(&args("ssh off")), Some(Verb::Ssh(false)));
        assert_eq!(
            parse(&args("rfkill bluetooth on")),
            Some(Verb::Rfkill(Radio::Bluetooth, true))
        );
        assert_eq!(parse(&args("power reboot")), Some(Verb::Power(true)));
        for bad in [
            "",
            "logs",
            "logs /etc/passwd",
            "ssh maybe",
            "rfkill nfc on",
            "power",
            "ssh on extra",
            "password",
        ] {
            assert_eq!(parse(&args(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn each_level_writes_its_storage() {
        assert!(logging_conf("off").contains("Storage=none"));
        assert!(logging_conf("ram").contains("Storage=volatile"));
        assert!(logging_conf("persistent").contains("Storage=persistent"));
    }
}
