//! Syntax colour, a line at a time. Each language is a small lexer that
//! takes the state the line starts in (inside a block comment, a string
//! that runs on) and gives back spans and the state the next line starts
//! in. Only lines on screen are lexed; [`Cache`] finds the state a screen
//! starts in from a checkpoint, or from a fixed distance above, so a
//! gigabyte file costs the same as a short one.

mod conf;
mod diff;
mod markup;
mod md;
mod table;

use crate::buf::Buffer;
pub use gh0st_app::gfx::theme::Syntax as Role;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// A run of one colour: it ends at byte `end` of the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub end: u32,
    pub role: Role,
}

/// Bytes of a line that are lexed. Past this a long line is plain: a
/// minified file's one line costs no more than a screen's worth.
pub const LINE_MAX: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Plain,
    Shell,
    Python,
    Rust,
    C,
    JavaScript,
    Go,
    Lua,
    Json,
    Css,
    Sql,
    Make,
    Dockerfile,
    /// INI, systemd units, desktop files and other key = value files.
    Conf,
    Toml,
    Yaml,
    Markdown,
    Diff,
    /// HTML, XML, SVG.
    Markup,
}

impl Lang {
    pub const ALL: [Lang; 19] = [
        Lang::Plain,
        Lang::Shell,
        Lang::Python,
        Lang::Rust,
        Lang::C,
        Lang::JavaScript,
        Lang::Go,
        Lang::Lua,
        Lang::Json,
        Lang::Css,
        Lang::Sql,
        Lang::Make,
        Lang::Dockerfile,
        Lang::Conf,
        Lang::Toml,
        Lang::Yaml,
        Lang::Markdown,
        Lang::Diff,
        Lang::Markup,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Lang::Plain => "Text",
            Lang::Shell => "Shell",
            Lang::Python => "Python",
            Lang::Rust => "Rust",
            Lang::C => "C",
            Lang::JavaScript => "JavaScript",
            Lang::Go => "Go",
            Lang::Lua => "Lua",
            Lang::Json => "JSON",
            Lang::Css => "CSS",
            Lang::Sql => "SQL",
            Lang::Make => "Makefile",
            Lang::Dockerfile => "Dockerfile",
            Lang::Conf => "Config",
            Lang::Toml => "TOML",
            Lang::Yaml => "YAML",
            Lang::Markdown => "Markdown",
            Lang::Diff => "Diff",
            Lang::Markup => "Markup",
        }
    }

    /// The line comment a toggle adds, if the language has one.
    pub fn comment(self) -> Option<&'static str> {
        match self {
            Lang::Rust | Lang::C | Lang::JavaScript | Lang::Go => Some("//"),
            Lang::Shell
            | Lang::Python
            | Lang::Make
            | Lang::Dockerfile
            | Lang::Conf
            | Lang::Toml
            | Lang::Yaml => Some("#"),
            Lang::Lua | Lang::Sql => Some("--"),
            _ => None,
        }
    }
}

/// The language of the file at `path`, from its name, then its first line.
pub fn detect(path: &Path, first_line: &[u8]) -> Lang {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let by_name = match name.as_str() {
        "makefile" | "gnumakefile" => Some(Lang::Make),
        "dockerfile" | "containerfile" => Some(Lang::Dockerfile),
        ".bashrc" | ".profile" | ".bash_profile" | ".zshrc" | ".bash_aliases" | ".bash_logout" => {
            Some(Lang::Shell)
        }
        ".gitconfig" | ".editorconfig" | ".npmrc" => Some(Lang::Conf),
        "cargo.lock" => Some(Lang::Toml),
        _ => None,
    };
    if let Some(l) = by_name {
        return l;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let by_ext = match ext {
        // Text by name, whatever its first line says.
        "txt" | "text" | "log" | "csv" | "tsv" => return Lang::Plain,
        "sh" | "bash" | "zsh" | "ksh" | "ash" | "dash" => Lang::Shell,
        "py" | "pyw" | "pyi" => Lang::Python,
        "rs" => Lang::Rust,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "ino" | "m" => Lang::C,
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "mts" | "cts" => Lang::JavaScript,
        "go" => Lang::Go,
        "lua" => Lang::Lua,
        "json" | "jsonc" | "json5" | "geojson" | "webmanifest" => Lang::Json,
        "css" | "scss" | "less" => Lang::Css,
        "sql" => Lang::Sql,
        "mk" | "mak" => Lang::Make,
        "dockerfile" => Lang::Dockerfile,
        "ini" | "cfg" | "conf" | "service" | "socket" | "timer" | "mount" | "target" | "path"
        | "desktop" | "network" | "netdev" | "link" | "slice" | "scope" | "automount" | "swap"
        | "palette" | "properties" => Lang::Conf,
        "toml" => Lang::Toml,
        "yml" | "yaml" => Lang::Yaml,
        "md" | "markdown" | "mdown" => Lang::Markdown,
        "diff" | "patch" => Lang::Diff,
        "html" | "htm" | "xhtml" | "xml" | "svg" | "plist" | "xsd" | "xsl" | "rss" | "atom"
        | "ui" | "glade" => Lang::Markup,
        _ => Lang::Plain,
    };
    if by_ext != Lang::Plain {
        return by_ext;
    }
    shebang(first_line).unwrap_or(Lang::Plain)
}

fn shebang(line: &[u8]) -> Option<Lang> {
    let rest = line.strip_prefix(b"#!")?;
    let s = String::from_utf8_lossy(rest);
    let mut words = s.split_whitespace();
    let mut prog = words.next()?.rsplit('/').next()?.to_string();
    if prog == "env" {
        prog = words.find(|w| !w.starts_with('-'))?.to_string();
    }
    let prog = prog.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    match prog {
        "sh" | "bash" | "dash" | "zsh" | "ash" | "ksh" | "busybox" => Some(Lang::Shell),
        "python" => Some(Lang::Python),
        "node" | "deno" | "bun" => Some(Lang::JavaScript),
        "lua" | "luajit" => Some(Lang::Lua),
        "make" => Some(Lang::Make),
        _ => None,
    }
}

/// Lexes `line` (without its newline) starting in `state`. Spans go to
/// `out`, which is cleared first; they cover the line up to [`LINE_MAX`].
/// Returns the state the next line starts in.
pub fn lex(lang: Lang, state: u8, line: &[u8], out: &mut Vec<Span>) -> u8 {
    out.clear();
    let line = &line[..line.len().min(LINE_MAX)];
    let mut o = Out { spans: out };
    match lang {
        Lang::Plain => {
            o.push(line.len(), Role::Plain);
            0
        }
        Lang::Markdown => md::lex(state, line, &mut o),
        Lang::Diff => diff::lex(line, &mut o),
        Lang::Markup => markup::lex(state, line, &mut o),
        Lang::Conf => conf::ini(line, &mut o),
        Lang::Toml => conf::toml(state, line, &mut o),
        Lang::Yaml => conf::yaml(line, &mut o),
        _ => table::lex(table::spec(lang), state, line, &mut o),
    }
}

/// Spans being built: a run of the same role grows the last span.
pub(crate) struct Out<'a> {
    spans: &'a mut Vec<Span>,
}

impl Out<'_> {
    /// The line is `role` up to byte `end`.
    pub fn push(&mut self, end: usize, role: Role) {
        let end = end as u32;
        if end <= self.spans.last().map_or(0, |s| s.end) {
            return;
        }
        match self.spans.last_mut() {
            Some(s) if s.role == role => s.end = end,
            _ => self.spans.push(Span { end, role }),
        }
    }

    /// Recolours the span that ends at `end`, as a string turns out to be
    /// a key once its colon shows.
    pub fn recolor_last(&mut self, role: Role) {
        if let Some(s) = self.spans.last_mut() {
            s.role = role;
        }
    }
}

/// Lines between checkpoints.
const EVERY: u32 = 256;
/// How far up a screen's state is looked for when no checkpoint is near:
/// past this, the screen starts plain. A block comment longer than this
/// that opens above may show uncoloured until scrolled through.
const LOOK_BACK: u32 = 512;
/// Checkpoints kept before they are all dropped.
const MARKS_MAX: usize = 1 << 16;

/// The state each line starts in, found from checkpoints.
pub struct Cache {
    lang: Lang,
    marks: BTreeMap<u64, u8>,
    spans: Vec<Span>,
    line: Vec<u8>,
    /// Lines lexed to find states, for tests and the bench.
    pub lexed: u64,
}

impl Cache {
    pub fn new(lang: Lang) -> Cache {
        Cache {
            lang,
            marks: BTreeMap::new(),
            spans: Vec::new(),
            line: Vec::new(),
            lexed: 0,
        }
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// Text changed at `at`: states of lines after it may have too.
    pub fn edited(&mut self, at: u64) {
        let _ = self.marks.split_off(&(at + 1));
    }

    /// Everything changed: a reload, another language.
    pub fn clear(&mut self, lang: Lang) {
        self.lang = lang;
        self.marks.clear();
    }

    /// The state the line starting at `start` begins in.
    pub fn state_at(&mut self, b: &mut Buffer, start: u64) -> io::Result<u8> {
        if matches!(
            self.lang,
            Lang::Plain | Lang::Diff | Lang::Conf | Lang::Yaml
        ) {
            // Lines in these stand alone.
            return Ok(0);
        }
        if let Some(&s) = self.marks.get(&start) {
            return Ok(s);
        }
        // The nearest line start above that is close enough to lex from.
        let mut top = start;
        let mut n = 0;
        while n < LOOK_BACK && top > 0 {
            if self.marks.contains_key(&top) {
                break;
            }
            top = b.line_start(top - 1)?;
            n += 1;
        }
        let mut state = self.marks.get(&top).copied().unwrap_or(0);
        let mut at = top;
        let mut since = 0;
        while at < start {
            let end = b.line_end(at)?;
            let take = (end - at).min(LINE_MAX as u64);
            self.line.clear();
            let line = &mut self.line;
            b.for_each(at..at + take, |c| {
                line.extend_from_slice(c);
                true
            })?;
            state = lex(self.lang, state, &self.line, &mut self.spans);
            self.lexed += 1;
            at = (end + 1).min(b.len());
            since += 1;
            if since % EVERY == 0 && at < start {
                self.mark(at, state);
            }
            if end >= b.len() {
                break;
            }
        }
        self.mark(start, state);
        Ok(state)
    }

    fn mark(&mut self, at: u64, state: u8) {
        if self.marks.len() >= MARKS_MAX {
            self.marks.clear();
        }
        self.marks.insert(at, state);
    }
}

#[cfg(test)]
mod tests;
