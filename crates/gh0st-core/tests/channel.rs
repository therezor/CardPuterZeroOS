//! The channel comes from the edge source, not the last check: a change
//! starts a check that can wait on apt or fail.

use gh0st_core::update::State;

#[test]
fn the_channel_is_the_source_not_the_last_check() {
    let dir = std::env::temp_dir().join(format!("gh0st-channel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("state"), "checked=1\noutcome=ok\nchannel=edge\n").unwrap();
    let edge = dir.join("edge.sources");
    // SAFETY: this binary runs one test; nothing else reads the environment.
    unsafe {
        std::env::set_var("GH0ST_UPDATE_STATE", &dir);
        std::env::set_var("GH0ST_UPDATE_EDGE", &edge);
    }
    assert_eq!(
        State::load().channel,
        "stable",
        "switched back since the check"
    );
    std::fs::write(&edge, "Suites: edge\n").unwrap();
    assert_eq!(State::load().channel, "edge");
    std::fs::remove_dir_all(&dir).unwrap();
}
