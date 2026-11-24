fn main() {
    gh0st_app::run(gh0st_keyring::app::Home::new(
        gh0st_keyring::backend::connect(),
    ));
}
