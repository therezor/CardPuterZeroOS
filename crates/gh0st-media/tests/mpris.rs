//! MPRIS on a real session bus, when there is one (CI, the image
//! container under dbus-run-session): another client reads the status and
//! presses buttons, as playerctl does.

use gh0st_app::dbus::{Conn, Kind, Message, Value, bus};
use gh0st_media::mpris::{self, Control, Mpris, State, Status};

fn ask(
    client: &mut Conn,
    player: &mut Mpris,
    state: &State,
    m: Message,
) -> (Message, Vec<Control>) {
    let serial = client.send(m).unwrap();
    client.flush().unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut controls = Vec::new();
    while std::time::Instant::now() < end {
        controls.extend(player.ready(state));
        let _ = client.fill();
        while let Ok(Some(r)) = client.recv() {
            if matches!(r.kind, Kind::Return | Kind::Error) && r.reply_serial == Some(serial) {
                return (r, controls);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("no reply");
}

#[test]
fn a_client_sees_the_music_and_presses_its_buttons() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return;
    }
    let state = State {
        status: Status::Playing,
        title: "Test Tone".into(),
        length_ms: 20_000,
        track: 1,
        ..State::default()
    };
    let Some(mut player) = Mpris::start(&state) else {
        panic!("the name was taken");
    };
    let mut client = Conn::session().unwrap();
    let get = Message::call(mpris::NAME, mpris::PATH, bus::PROPERTIES, "Get")
        .arg(Value::str("org.mpris.MediaPlayer2.Player"))
        .arg(Value::str("PlaybackStatus"));
    let (r, _) = ask(&mut client, &mut player, &state, get);
    assert_eq!(r.body[0].inner().as_str(), Some("Playing"));
    let press = Message::call(
        mpris::NAME,
        mpris::PATH,
        "org.mpris.MediaPlayer2.Player",
        "PlayPause",
    );
    let (r, c) = ask(&mut client, &mut player, &state, press);
    assert!(r.error_name.is_none());
    assert_eq!(c, [Control::PlayPause]);
    drop(player);
    // Gone with the music: the name is free again.
    let has = Message::call(
        bus::NAME,
        "/org/freedesktop/DBus",
        bus::NAME,
        "NameHasOwner",
    )
    .arg(Value::str(mpris::NAME));
    let r = client.call(has, 2000).unwrap();
    assert_eq!(r.body[0], Value::Bool(false));
}
