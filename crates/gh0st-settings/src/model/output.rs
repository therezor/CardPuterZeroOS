//! Where sound plays: the built-in speaker or a Bluetooth device. pactl's
//! output read into Settings state; the jobs run it.

/// The default sink, then every sink, between markers.
pub const QUERY: &str = "pactl get-default-sink; echo @@; pactl list sinks";

/// One place sound can play.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// The sink's name, for `pactl set-default-sink`.
    pub name: String,
    /// What the row shows.
    pub label: String,
}

/// The sinks a person can pick, and the default's name. A paused app's
/// null sink (`gh0st audio`) is not one.
pub fn parse(out: &str) -> (Vec<Output>, String) {
    let (default, list) = out.split_once("@@").unwrap_or(("", out));
    let mut v: Vec<Output> = Vec::new();
    let mut name = None;
    for line in list.lines().map(str::trim) {
        if let Some(n) = line.strip_prefix("Name: ") {
            name = Some(n.to_string());
        } else if let Some(d) = line.strip_prefix("Description: ") {
            let Some(n) = name.take() else {
                continue;
            };
            let label = if n.starts_with("alsa_output.") {
                "Speaker".to_string()
            } else if n.starts_with("bluez_output.") {
                d.to_string()
            } else {
                continue;
            };
            v.push(Output { name: n, label });
        }
    }
    (v, default.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "bluez_output.D8_37_3B_35_E0_81.1\n@@\n\
Sink #41\n\tState: IDLE\n\tName: alsa_output.platform-sound.stereo-fallback\n\
\tDescription: Built-in Audio Stereo\n\
Sink #77\n\tName: gh0st-park-812\n\tDescription: Null Output\n\
Sink #90\n\tName: bluez_output.D8_37_3B_35_E0_81.1\n\tDescription: JBL Xtreme 3\n";

    #[test]
    fn the_speaker_and_bluetooth_but_no_parked_app() {
        let (v, default) = parse(OUT);
        let labels: Vec<&str> = v.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, ["Speaker", "JBL Xtreme 3"]);
        assert_eq!(default, "bluez_output.D8_37_3B_35_E0_81.1");
    }
}
