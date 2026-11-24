use gh0st_app::portal::{self, PickMode, Question};
use gh0st_files::app::{Files, Options, Picker};
use gh0st_files::pick::{Filter, Mode, Pattern, Spec};

fn main() {
    // ssh runs this binary as its askpass program, with the socket to call.
    if let Ok(path) = std::env::var(gh0st_files::worker::askpass::ENV) {
        std::process::exit(gh0st_files::worker::askpass::client(&path));
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--worker") => std::process::exit(gh0st_files::worker::main()),
        // The shell runs us as an app's file chooser (docs/portal.md).
        Some("--pick") => {
            let Some(id) = args.get(1).and_then(|i| i.parse::<u64>().ok()) else {
                eprintln!("usage: gh0st-files --pick <id>");
                std::process::exit(2);
            };
            let (mut conn, q) = match portal::take(id) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("gh0st-files: no question for {id}: {e}");
                    std::process::exit(1);
                }
            };
            let mut o = Options::system();
            o.pick = Some(Picker {
                spec: spec(q),
                done: Box::new(move |paths, filter| {
                    if let Err(e) = portal::done(&mut conn, id, paths.as_deref(), filter) {
                        eprintln!("gh0st-files: answer for {id}: {e}");
                    }
                }),
            });
            gh0st_app::run(Files::with(o));
        }
        _ => gh0st_app::run(Files::new()),
    }
}

fn spec(q: Question) -> Spec {
    let mode = match q.mode {
        PickMode::Open => Mode::Open,
        PickMode::Save => Mode::Save,
        PickMode::Folder => Mode::Folder,
    };
    let mut s = Spec::new(mode, &q.app);
    s.multiple = q.multiple;
    s.accept = (!q.accept.is_empty()).then_some(q.accept);
    s.filters = q
        .filters
        .into_iter()
        .map(|(name, pats)| Filter {
            name,
            patterns: pats
                .into_iter()
                .map(|(kind, p)| {
                    if kind == 1 {
                        Pattern::Mime(p)
                    } else {
                        Pattern::Glob(p)
                    }
                })
                .collect(),
        })
        .collect();
    s.filter = q.filter.filter(|&f| f < s.filters.len());
    s.name = q.name;
    s.folder = q.folder;
    s
}
