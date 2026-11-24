//! The Keyring app on the real Secret Service, from the real keys.

mod common;

use common::fake::Fake;
use gh0st_app::testing::Driver;
use gh0st_keyring::app::Home;
use gh0st_keyringd::secret::Answer;

fn step(d: &mut Driver, keys: &str) {
    d.keys(keys);
    d.wait(100);
}

#[test]
fn add_show_delete_and_revoke() {
    let mut f = Fake::new("flow");
    let gh = f.seed(
        "exe:/usr/bin/git",
        "GitHub token",
        &[("service", "github.com")],
        b"ghp",
    );
    f.approve("exe:/usr/bin/python3", &gh);
    let kr = f.shared();
    let mut d = Driver::new(Home::new(kr.clone()));
    d.wait(50);

    // Add one by hand: it is the app's own.
    step(&mut d, "7");
    step(&mut d, "type GitLab\ntab\ntype s3cret\nenter");
    assert_eq!(d.toast(), Some("Saved"));
    let items = kr.borrow_mut().items();
    let ours = items.iter().find(|i| i.label == "GitLab").expect("stored");
    assert_eq!(ours.app, "keyring");

    // Revoke python's access to the GitHub token.
    assert_eq!(kr.borrow_mut().access().len(), 1);
    step(&mut d, "x|x".replace('|', "\n").as_str());
    step(&mut d, "enter");
    step(&mut d, "6");
    assert!(kr.borrow_mut().access().is_empty());
    step(&mut d, "esc");

    // Delete the GitHub token: the confirm, then the keyring's prompt.
    step(&mut d, "f\nf\nenter");
    step(&mut d, "enter");
    step(&mut d, "7");
    assert!(d.dialog().is_some(), "asks first");
    step(&mut d, "enter");
    let left: Vec<String> = kr
        .borrow_mut()
        .items()
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert_eq!(left, ["GitLab"]);
    assert!(d.lint().is_empty());
}

#[test]
fn a_denied_reveal_shows_nothing() {
    let mut f = Fake::new("deny");
    f.seed(
        "exe:/usr/bin/git",
        "GitHub token",
        &[("service", "github.com")],
        b"ghp",
    );
    // The password prompt is refused: Esc on the shell's screen.
    f.password = b"wrong".to_vec();
    f.answer = Answer::Deny;
    f.svc.verified_prompts = false;
    let kr = f.shared();
    let mut d = Driver::new(Home::new(kr.clone()));
    d.wait(50);
    step(&mut d, "enter");
    step(&mut d, "enter");
    step(&mut d, "5");
    assert_ne!(d.toast(), None);
    assert!(d.lint().is_empty());
}

#[test]
fn locked_then_unlocked_from_the_app() {
    let mut f = Fake::new("unlock");
    f.seed("exe:/usr/bin/git", "GitHub token", &[], b"ghp");
    f.svc.lock();
    let kr = f.shared();
    let mut d = Driver::new(Home::new(kr.clone()));
    d.wait(50);
    assert_eq!(
        kr.borrow_mut().state(),
        gh0st_keyring::backend::State::Locked
    );
    step(&mut d, "6");
    assert_eq!(kr.borrow_mut().state(), gh0st_keyring::backend::State::Open);
}
