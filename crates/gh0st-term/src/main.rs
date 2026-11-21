use gh0st_term::app::{Options, Term};

fn main() {
    gh0st_app::run(Term::new(Options::from_args(std::env::args().skip(1))));
}
