pub mod hosts;
pub mod media;
pub mod mouse;
pub mod pair;
pub mod profile;
pub mod session;
pub mod slides;
pub mod snippets;

use gh0st_app::Cx;

use crate::app::{Shared, Target};
use crate::store;

/// Keys go to the host at `address` from now on: Bluetooth on, any other
/// host let go, and an advert for this one to come back.
pub fn use_host(s: &mut Shared, address: &str, cx: &mut Cx) {
    s.set_target(Target::Bt(address.to_string()), cx);
    s.radio_on(cx);
    let path = store::path_of(address);
    if let Some(bt) = &mut s.bt {
        if let Some(other) = bt.connected().map(String::from)
            && other != path
        {
            bt.disconnect(&other);
        }
        if bt.connected() != Some(path.as_str()) {
            bt.reconnect(true);
        }
    }
}
