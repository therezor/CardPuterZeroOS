//! BlueZ's errors in words. `crate::bluez` talks to BlueZ; this only says
//! what went wrong.

/// A failed pair, connect, disconnect or forget of `name`, from BlueZ's
/// error name and text: what to do next, where there is something to do.
pub fn explain(err: &str, name: &str) -> String {
    let has = |words: &[&str]| words.iter().any(|w| err.contains(w));
    if has(&[
        "ConnectionAttemptFailed",
        "page-timeout",
        "Page Timeout",
        "Host is down",
        "DoesNotExist",
    ]) {
        format!("{name} didn't answer. Is it on and in pairing mode?")
    } else if has(&["AuthenticationFailed", "AuthenticationRejected", "Rejected"]) {
        format!("{name} refused to pair")
    } else if has(&["AuthenticationCanceled", "AuthenticationTimeout"]) {
        format!("{name} stopped pairing. Try again")
    } else if has(&["profile-unavailable"]) {
        format!("{name} has nothing this device can use")
    } else if has(&["InProgress", "Busy"]) {
        "Bluetooth is busy. Try again".into()
    } else if has(&["NotReady"]) {
        "Bluetooth is off".into()
    } else {
        format!("Couldn't reach {name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_say_what_to_do() {
        let say = |err| explain(err, "JBL");
        assert_eq!(
            say("org.bluez.Error.ConnectionAttemptFailed: Page Timeout"),
            "JBL didn't answer. Is it on and in pairing mode?"
        );
        assert_eq!(
            say("org.bluez.Error.Failed: br-connection-page-timeout"),
            "JBL didn't answer. Is it on and in pairing mode?"
        );
        assert_eq!(
            say("org.bluez.Error.AuthenticationRejected: Authentication Rejected"),
            "JBL refused to pair"
        );
        assert_eq!(
            say("org.bluez.Error.Failed: br-connection-profile-unavailable"),
            "JBL has nothing this device can use"
        );
        assert_eq!(
            say("org.bluez.Error.Failed: Input/output error"),
            "Couldn't reach JBL"
        );
    }
}
