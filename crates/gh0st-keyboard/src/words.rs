//! What the app says, in one place.

pub const APP: &str = "Keyboard";
pub const USB: &str = "USB cable";
pub const CAPS: &str = "CAPS";

pub const RADIO_ON: &str = "Turning Bluetooth on\u{2026}";
pub const RADIO_FAILED: &str = "Bluetooth didn't turn on";

// Hosts
pub const PAIR_NOTE: &str = "Pair adds a computer or phone over Bluetooth.";
pub const FORGET: &str = "Forget";
pub const NO_BT: &str = "Bluetooth isn't working";
pub const BT_OFF: &str = "Bluetooth off";
pub const CONNECTED: &str = "Connected";
pub const WAITING: &str = "Waiting";
pub const BT_NAME: &str = "Bluetooth name";
pub const BT_NAME_NOTE: &str = "Hosts see it while pairing.";
pub const FORGOT: &str = "Forgotten. Pair it again to use it.";

pub fn usb_note(s: crate::link::UsbState) -> &'static str {
    use crate::link::UsbState::*;
    match s {
        Off => "Off. Enter turns it on",
        Starting => "Starting\u{2026}",
        HostMode => "Set the side switch to Slave",
        NoCable => "Plug into a computer or phone",
        Connected => "Typing over the cable",
        Asleep => "The host is asleep",
        Failed => "Didn't start. Enter tries again",
    }
}

// Pairing
pub const PAIR: &str = "Pair";
pub const PAIR_WAIT: &str = "Waiting for a computer or phone";
pub fn pair_how(name: &str) -> String {
    format!("On it, open Bluetooth and pick {name}.")
}
pub const PAIR_ENDED: &str = "Nobody paired";
pub const PAIR_AGAIN: &str = "It stopped looking. Enter looks again.";
pub const PAIR_CANCELLED: &str = "The host stopped pairing";
pub fn pair_confirm(host: &str) -> String {
    format!("Does {host} show this code?")
}
pub fn pair_show(host: &str) -> String {
    format!("Type this code on {host}, then Enter there.")
}
pub fn pair_allow(host: &str) -> String {
    format!("Pair with {host}?")
}
pub fn pair_enter(host: &str) -> String {
    format!("Type the code {host} shows")
}
pub const THE_HOST: &str = "the host";

// A host's page
pub const START_TYPING: &str = "Start typing";
pub const NAME: &str = "Name";
pub const SYSTEM: &str = "System";
pub const STAY: &str = "Stay connected behind apps";
pub const USB_PROFILE_ABOUT: &str = "System sets what Alt sends (Cmd on a Mac or an iPhone) and the shortcuts, for whatever the USB cable plugs into.";
pub const PROFILE_ABOUT: &str = "System sets what Alt sends (Cmd on a Mac or an iPhone) and the shortcuts.
Stay connected keeps the host linked while another app is in front. It is off for a phone, so the phone's own keyboard comes back.";
pub fn paired(host: &str) -> String {
    format!("Paired with {host}")
}

// The session
pub const MODES: [&str; 5] = ["Type", "Mouse", "Media", "Slides", "Macros"];
pub const MODE_ICONS: [&str; 5] = ["keyboard", "mouse", "music", "presentation", "file-text"];
pub fn waiting_for(host: &str) -> String {
    format!("Waiting for {host}")
}
pub const WAKE_IT: &str = "Wake it and turn its Bluetooth on.";
pub fn tap_on_phone(me: &str) -> String {
    format!("On the phone: Settings, Bluetooth, tap {me}.")
}
pub const NOT_BACK: &str = "It didn't come back. Enter tries again.";
pub const SLAVE_NOTE: &str = "Slave turns off USB-A, Ethernet and the other USB-C.";
pub const CABLE_NOTE: &str = "Use the right-hand USB-C port.";
pub fn typing_on(host: &str) -> String {
    format!("Typing on {host}")
}
pub const LATCHED: &str = "with the next key";

pub const TYPE_ABOUT: &str = "Every key goes to the host.
Esc goes back. Fn Esc sends Esc.
Fn Space picks a mode.
Fn 1 to 0: F1 to F10. Fn O, P: F11, F12.
Fn F Z X C: the arrows.
Fn L, M: page up, down. Fn N: End.
Fn Tab: Home. Fn Bksp: Delete.
Fn Q, W, E: play or pause, previous, next.
Fn then Alt: Win, Super or Option with the next key. Twice: alone.
On a Mac or an iPhone, Alt is Cmd.";

pub const MOUSE_ABOUT: &str = "F Z X C or the arrows move the pointer: hold to go faster.
Shift moves faster.
Space or Enter clicks; hold Space to drag.
Bksp is the right button, P the middle one.
R and V scroll.
Fn Space picks a mode.
An iPhone shows the pointer only with AssistiveTouch on: Settings, Accessibility, Touch.";

pub const IPHONE_POINTER: &str = "iPhone: turn on AssistiveTouch for a pointer";
pub const SHOW_TYPED: &str = "Show what I type";
pub const UNLINK: &str = "Unlink";
pub const MODES_LABEL: &str = "Modes";

// Media
pub const PLAY: &str = "Play or pause";
pub const PREV: &str = "Prev";
pub const NEXT: &str = "Next";
pub const MUTE: &str = "Mute";
pub const VOLUME_UP: &str = "Volume up";
pub const VOLUME_DOWN: &str = "Volume down";
pub const SHUTTER: &str = "Camera shutter";

// Slides
pub const BLACK: &str = "Black";
pub const START: &str = "Start";
pub const TIMER: &str = "Timer";
pub const NEXT_SLIDE: &str = "Next slide";
pub const PREV_SLIDE: &str = "Previous slide";
pub const RESET_TIMER: &str = "Reset timer";
pub fn slide(n: i32) -> String {
    format!("Slide {n}")
}

// Snippets
pub const NEW: &str = "New";
pub const DELETE: &str = "Delete";
pub const SNIPPET_NAME: &str = "Macro name";
pub const SNIPPET_TEXT: &str = "Text to type";
pub fn typing_progress(done: usize, all: usize) -> String {
    format!("Typing {done} of {all}")
}
pub fn skipped(n: usize) -> String {
    format!("{n} characters a US keyboard can't type were left out")
}
pub const NOT_LIVE: &str = "Not connected";
