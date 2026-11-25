//! `gh0st audio`: parks the sound of an app the shell paused. A stopped app
//! leaves its stream open and uncorked, so PipeWire keeps the codec and the
//! I2S DMA running and wakes about 1,700 times a second for data that
//! never comes. Parking moves the group's streams to a null sink of its
//! own, and the real device then suspends. The shell runs this as a child,
//! so `pactl` never blocks its loop.
//!
//! ```text
//! gh0st audio park PGID     after the shell stops the group
//! gh0st audio unpark PGID   when it resumes, or once the group is gone
//! gh0st audio clear         at shell start: what a previous shell left
//! ```
//!
//! The null sink forces a quantum of 8192 frames: a parked stream still
//! belongs to a graph, and at the app's own quantum that graph would cycle
//! hundreds of times a second. Its name carries the process group, so a
//! target WirePlumber saved for the app never matches a later sink.
//! Unparking unloads the sink; PipeWire then moves the streams back to the
//! default device. `state.restore-props=false` keeps each sink out of
//! WirePlumber's state file. The image turns off restoring saved targets,
//! so a new stream never lands on a live parked sink.

use gh0st_core::{child, procfs};
use std::process::Command;

const PREFIX: &str = "gh0st-parked-";
/// About 170 ms at 48 kHz: six graph cycles a second while parked.
const PROPS: &str = "sink_properties='node.force-quantum=8192 \
                     state.restore-props=false device.description=Parked'";

pub fn main(args: &[String]) -> i32 {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["park", pgid] => pgid.parse().map_or(2, |p| {
            park(p);
            0
        }),
        ["unpark", pgid] => pgid.parse().map_or(2, |p| {
            unpark(p);
            0
        }),
        ["clear"] => {
            unload_matching(PREFIX);
            0
        }
        _ => {
            eprintln!("usage: gh0st audio park|unpark PGID | clear");
            2
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Playback,
    Capture,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stream {
    kind: Kind,
    index: u32,
    corked: bool,
    pid: Option<i32>,
}

fn sink_name(pgid: i32) -> String {
    format!("{PREFIX}{pgid}")
}

fn park(pgid: i32) {
    let members: Vec<i32> = procfs::processes()
        .filter(|p| p.pgrp == pgid)
        .map(|p| p.pid)
        .collect();
    // Without an audio library loaded the group has no stream, and asking
    // pactl would start pipewire-pulse for nothing.
    if !members.iter().any(|&pid| uses_audio(pid)) {
        return;
    }
    let mut found = Vec::new();
    for (what, kind) in [
        ("sink-inputs", Kind::Playback),
        ("source-outputs", Kind::Capture),
    ] {
        if let Some(text) = pactl(&["list", what]) {
            found.extend(parse_streams(&text, kind));
        }
    }
    let ours: Vec<Stream> = found
        .into_iter()
        .filter(|s| !s.corked && s.pid.is_some_and(|p| members.contains(&p)))
        .collect();
    if ours.is_empty() {
        return;
    }
    let name = sink_name(pgid);
    if modules_matching(&name).is_empty() {
        let arg = format!("sink_name={name}");
        if pactl(&["load-module", "module-null-sink", &arg, PROPS]).is_none() {
            return;
        }
    }
    let monitor = format!("{name}.monitor");
    for s in ours {
        let index = s.index.to_string();
        let _ = match s.kind {
            Kind::Playback => pactl(&["move-sink-input", &index, &name]),
            Kind::Capture => pactl(&["move-source-output", &index, &monitor]),
        };
    }
    // Resumed while this ran: the shell's unpark may have come first.
    if !child::group_stopped(pgid, 0) {
        unpark(pgid);
    }
}

fn unpark(pgid: i32) {
    unload_matching(&sink_name(pgid));
}

/// Unloads our null sinks whose name is `name`, or starts with it when it
/// ends in `-`. Only while pipewire-pulse runs: otherwise none exists.
fn unload_matching(name: &str) {
    if !running("pipewire-pulse") {
        return;
    }
    for m in modules_matching(name) {
        let _ = pactl(&["unload-module", &m]);
    }
}

fn modules_matching(name: &str) -> Vec<String> {
    pactl(&["list", "short", "modules"])
        .map(|t| parse_modules(&t, name))
        .unwrap_or_default()
}

/// Module indexes from `pactl list short modules` for null sinks named
/// `name`, or with that prefix when it ends in `-`.
fn parse_modules(text: &str, name: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let (index, module, args) = (f.next()?, f.next()?, f.next()?);
            let sink = args
                .split_whitespace()
                .find_map(|a| a.strip_prefix("sink_name="))?;
            let hit = if name.ends_with('-') {
                sink.starts_with(name)
            } else {
                sink == name
            };
            (module == "module-null-sink" && hit).then(|| index.to_string())
        })
        .collect()
}

/// Reads the long form of `pactl list sink-inputs` or `source-outputs`.
fn parse_streams(text: &str, kind: Kind) -> Vec<Stream> {
    let header = match kind {
        Kind::Playback => "Sink Input #",
        Kind::Capture => "Source Output #",
    };
    let mut out: Vec<Stream> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(n) = line.strip_prefix(header) {
            if let Ok(index) = n.parse() {
                out.push(Stream {
                    kind,
                    index,
                    corked: false,
                    pid: None,
                });
            }
        } else if let Some(s) = out.last_mut() {
            if let Some(v) = line.strip_prefix("Corked:") {
                s.corked = v.trim() == "yes";
            } else if let Some(v) = line.strip_prefix("application.process.id = ") {
                s.pid = v.trim_matches('"').parse().ok();
            }
        }
    }
    out
}

/// The process has libpulse or libpipewire mapped.
fn uses_audio(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/maps"))
        .is_ok_and(|m| m.contains("/libpulse") || m.contains("/libpipewire-"))
}

fn running(comm: &str) -> bool {
    procfs::processes().any(|p| {
        std::fs::read_to_string(format!("/proc/{}/comm", p.pid)).is_ok_and(|c| c.trim() == comm)
    })
}

/// Runs pactl in the C locale, so its labels are the ones parsed here.
/// Its output, or None when it failed.
fn pactl(args: &[&str]) -> Option<String> {
    let out = Command::new("pactl")
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SINK_INPUTS: &str = include_str!("testdata/sink-inputs.txt");
    const SOURCE_OUTPUTS: &str = include_str!("testdata/source-outputs.txt");
    const MODULES: &str = include_str!("testdata/modules.txt");

    #[test]
    fn reads_each_stream_with_its_pid_and_cork() {
        // The first block is the device's own output, Music playing.
        let s = parse_streams(SINK_INPUTS, Kind::Playback);
        let want = |index, corked, pid| Stream {
            kind: Kind::Playback,
            index,
            corked,
            pid: Some(pid),
        };
        assert_eq!(s, vec![want(284, false, 19855), want(287, true, 20110)]);
    }

    // Written by hand in the layout of the device's sink-inputs.
    #[test]
    fn reads_capture_streams_under_their_own_header() {
        let s = parse_streams(SOURCE_OUTPUTS, Kind::Capture);
        assert_eq!(s.len(), 1);
        assert_eq!(
            (s[0].index, s[0].pid, s[0].corked),
            (211, Some(17071), false)
        );
        assert!(parse_streams(SOURCE_OUTPUTS, Kind::Playback).is_empty());
    }

    #[test]
    fn finds_our_null_sinks_by_name_or_prefix() {
        // The device's own list: native modules span several lines.
        assert_eq!(
            parse_modules(MODULES, "gh0st-parked-19854"),
            vec!["536870916"]
        );
        assert_eq!(
            parse_modules(MODULES, PREFIX),
            vec!["536870916", "536870917"]
        );
        // Another null sink, and a name that only starts like ours.
        assert!(parse_modules(MODULES, "gh0st-parked-1985").is_empty());
    }

    #[test]
    fn usage_errors_exit_2() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(main(&args(&["park"])), 2);
        assert_eq!(main(&args(&["park", "x"])), 2);
        assert_eq!(main(&args(&["louder"])), 2);
    }
}
