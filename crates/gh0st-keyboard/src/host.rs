//! What the app keeps about each computer or phone it types on.

/// The host's system. It decides what Alt sends, which key the latch gives,
/// and the built-in shortcuts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum System {
    #[default]
    Windows,
    Mac,
    Linux,
    ChromeOs,
    Android,
    Ios,
}

impl System {
    pub const ALL: [System; 6] = [
        System::Windows,
        System::Mac,
        System::Linux,
        System::ChromeOs,
        System::Android,
        System::Ios,
    ];

    /// On Apple systems Alt sends Command, and the latch gives Option.
    pub fn apple(self) -> bool {
        matches!(self, System::Mac | System::Ios)
    }

    /// A phone or tablet hides its on-screen keyboard while a keyboard is
    /// connected, so the app lets go of it when it leaves the front.
    pub fn handheld(self) -> bool {
        matches!(self, System::Android | System::Ios)
    }

    pub fn id(self) -> &'static str {
        match self {
            System::Windows => "windows",
            System::Mac => "mac",
            System::Linux => "linux",
            System::ChromeOs => "chromeos",
            System::Android => "android",
            System::Ios => "ios",
        }
    }

    pub fn from_id(id: &str) -> System {
        System::ALL
            .into_iter()
            .find(|s| s.id() == id)
            .unwrap_or_default()
    }

    pub fn name(self) -> &'static str {
        match self {
            System::Windows => "Windows",
            System::Mac => "macOS",
            System::Linux => "Linux",
            System::ChromeOs => "ChromeOS",
            System::Android => "Android",
            System::Ios => "iPhone, iPad",
        }
    }

    /// What the host calls the key the latch gives.
    pub fn latch_name(self) -> &'static str {
        match self {
            System::Mac | System::Ios => "Option",
            System::Windows => "Win",
            System::ChromeOs => "Search",
            System::Linux | System::Android => "Super",
        }
    }

    /// What the host calls Alt as sent.
    pub fn alt_name(self) -> &'static str {
        if self.apple() { "Cmd" } else { "Alt" }
    }

    /// A guess from what BlueZ says of the device: its name, icon and
    /// modalias. The user can change it on the host's page.
    pub fn guess(name: &str, icon: &str, modalias: &str) -> System {
        let lower = name.to_lowercase();
        if lower.contains("iphone") || lower.contains("ipad") {
            return System::Ios;
        }
        if ["macbook", "imac", "mac mini", "mac pro", "mac studio"]
            .iter()
            .any(|m| lower.contains(m))
        {
            return System::Mac;
        }
        // Phones that give no icon still give their name.
        const ANDROID: [&str; 16] = [
            "android",
            "pixel",
            "galaxy",
            "poco",
            "redmi",
            "xiaomi",
            "oneplus",
            "moto",
            "nokia",
            "oppo",
            "vivo",
            "realme",
            "honor",
            "huawei",
            "xperia",
            "nothing phone",
        ];
        if ANDROID.iter().any(|b| lower.contains(b)) {
            return System::Android;
        }
        let apple = modalias.contains("v004C") || modalias.contains("v05AC");
        match (icon, apple) {
            ("phone", true) => System::Ios,
            ("phone", false) => System::Android,
            (_, true) => System::Mac,
            _ => System::Windows,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for s in System::ALL {
            assert_eq!(System::from_id(s.id()), s);
        }
        assert_eq!(System::from_id("beos"), System::Windows);
    }

    #[test]
    fn guesses_from_bluez() {
        let g = System::guess;
        assert_eq!(g("", "phone", "bluetooth:v004Cp7213d1360"), System::Ios);
        assert_eq!(g("iPhone REZOR", "phone", ""), System::Ios);
        assert_eq!(g("Pixel 9", "phone", ""), System::Android);
        assert_eq!(g("POCO X7 Pro", "", ""), System::Android);
        assert_eq!(g("Alex's MacBook Air", "computer", ""), System::Mac);
        assert_eq!(g("", "computer", "usb:v05ACp0000"), System::Mac);
        assert_eq!(g("DESKTOP-1", "computer", ""), System::Windows);
    }
}
