//! One lexer for the C-like and script languages, driven by a table per
//! language: its comments, quotes, words and a few habits.
//!
//! State: 0 is plain code. 1 to 15 is a block comment that deep. 16 and
//! up is inside quote `(s - 16) / 16` of the table; one past the table's
//! quotes is a Rust raw string with `(s - 16) % 16` hashes.

use super::{Lang, Out, Role};

pub(super) struct Quote {
    open: &'static [u8],
    close: &'static [u8],
    /// Backslash escapes the next byte.
    escape: bool,
    /// Runs on past the end of the line.
    multiline: bool,
}

const fn q(open: &'static [u8], close: &'static [u8], escape: bool, multiline: bool) -> Quote {
    Quote {
        open,
        close,
        escape,
        multiline,
    }
}

pub(super) struct Spec {
    line: &'static [&'static [u8]],
    block: Option<(&'static [u8], &'static [u8])>,
    nested: bool,
    /// Longest first: `"""` before `"`.
    quotes: &'static [Quote],
    /// Sorted, for a binary search.
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    consts: &'static [&'static str],
    /// Bytes that may sit in a word besides letters, digits and `_`.
    word: &'static [u8],
    /// Words match whatever their case (SQL).
    fold: bool,
    /// A word starting with a capital is a type.
    caps_types: bool,
    /// A word before `(` is a function; in Rust a word before `!` too.
    calls: bool,
    /// `#` starts a preprocessor line (C), `#[` an attribute (Rust).
    hash: Hash,
    /// `@name` is an attribute or decorator.
    at_attr: bool,
    /// `$name`, `${…}` and `$(…)` are variables (shell, make).
    dollar: bool,
    /// `'a` is a lifetime unless it closes as a char (Rust).
    lifetimes: bool,
    /// `r"…"` and `r#"…"#` (Rust).
    raw: bool,
    /// String prefixes such as Python's `f"…"` and `rb'…'`.
    prefixes: &'static [&'static str],
    /// A string or word just before `:` is a key (JSON).
    keys: bool,
    /// A word at the start of a line before `:` is a target (make).
    targets: bool,
    /// A comment needs a blank or the line start before it (shell `#`).
    comment_at_word: bool,
}

const BASE: Spec = Spec {
    line: &[],
    block: None,
    nested: false,
    quotes: &[],
    keywords: &[],
    types: &[],
    consts: &[],
    word: &[],
    fold: false,
    caps_types: false,
    calls: false,
    hash: Hash::None,
    at_attr: false,
    dollar: false,
    lifetimes: false,
    raw: false,
    prefixes: &[],
    keys: false,
    targets: false,
    comment_at_word: false,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hash {
    None,
    Preproc,
    Attribute,
}

const DQ: Quote = q(b"\"", b"\"", true, false);
const SQ: Quote = q(b"'", b"'", true, false);

static RUST: Spec = Spec {
    line: &[b"//"],
    block: Some((b"/*", b"*/")),
    nested: true,
    quotes: &[q(b"\"", b"\"", true, true), q(b"b\"", b"\"", true, true)],
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
        "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
        "pub", "ref", "return", "self", "static", "struct", "super", "trait", "type", "union",
        "unsafe", "use", "where", "while", "yield",
    ],
    types: &[
        "Self", "bool", "char", "f32", "f64", "i128", "i16", "i32", "i64", "i8", "isize", "str",
        "u128", "u16", "u32", "u64", "u8", "usize",
    ],
    consts: &["None", "Some", "false", "true"],
    caps_types: true,
    calls: true,
    hash: Hash::Attribute,
    lifetimes: true,
    raw: true,
    ..BASE
};

static C: Spec = Spec {
    line: &[b"//"],
    block: Some((b"/*", b"*/")),
    quotes: &[DQ, SQ],
    keywords: &[
        "auto",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "constexpr",
        "continue",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "explicit",
        "extern",
        "for",
        "friend",
        "goto",
        "if",
        "inline",
        "namespace",
        "new",
        "noexcept",
        "operator",
        "private",
        "protected",
        "public",
        "register",
        "return",
        "sizeof",
        "static",
        "struct",
        "switch",
        "template",
        "this",
        "throw",
        "try",
        "typedef",
        "typename",
        "union",
        "using",
        "virtual",
        "volatile",
        "while",
    ],
    types: &[
        "bool", "char", "double", "float", "int", "int16_t", "int32_t", "int64_t", "int8_t",
        "long", "short", "signed", "size_t", "ssize_t", "uint16_t", "uint32_t", "uint64_t",
        "uint8_t", "unsigned", "void",
    ],
    consts: &["NULL", "false", "nullptr", "true"],
    calls: true,
    hash: Hash::Preproc,
    ..BASE
};

static JS: Spec = Spec {
    line: &[b"//"],
    block: Some((b"/*", b"*/")),
    quotes: &[DQ, SQ, q(b"`", b"`", true, true)],
    keywords: &[
        "abstract",
        "as",
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "declare",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "finally",
        "for",
        "from",
        "function",
        "get",
        "if",
        "implements",
        "import",
        "in",
        "instanceof",
        "interface",
        "let",
        "new",
        "of",
        "private",
        "protected",
        "public",
        "readonly",
        "return",
        "set",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "try",
        "type",
        "typeof",
        "var",
        "void",
        "while",
        "with",
        "yield",
    ],
    types: &[
        "any", "bigint", "boolean", "never", "number", "object", "string", "symbol", "unknown",
    ],
    consts: &["Infinity", "NaN", "false", "null", "true", "undefined"],
    word: b"$",
    caps_types: true,
    calls: true,
    at_attr: true,
    ..BASE
};

static GO: Spec = Spec {
    line: &[b"//"],
    block: Some((b"/*", b"*/")),
    quotes: &[DQ, SQ, q(b"`", b"`", false, true)],
    keywords: &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "type",
        "var",
    ],
    types: &[
        "any",
        "bool",
        "byte",
        "complex128",
        "complex64",
        "error",
        "float32",
        "float64",
        "int",
        "int16",
        "int32",
        "int64",
        "int8",
        "rune",
        "string",
        "uint",
        "uint16",
        "uint32",
        "uint64",
        "uint8",
        "uintptr",
    ],
    consts: &["false", "iota", "nil", "true"],
    calls: true,
    ..BASE
};

static PYTHON: Spec = Spec {
    line: &[b"#"],
    quotes: &[
        q(b"\"\"\"", b"\"\"\"", true, true),
        q(b"'''", b"'''", true, true),
        DQ,
        SQ,
    ],
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "case", "class", "continue", "def",
        "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in",
        "is", "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return", "self", "try",
        "while", "with", "yield",
    ],
    types: &[
        "bool", "bytes", "dict", "float", "int", "list", "object", "set", "str", "tuple", "type",
    ],
    consts: &["False", "None", "True"],
    caps_types: true,
    calls: true,
    at_attr: true,
    prefixes: &[
        "B", "BR", "Br", "F", "FR", "Fr", "R", "RB", "RF", "Rb", "Rf", "U", "b", "bR", "br", "f",
        "fR", "fr", "r", "rB", "rF", "rb", "rf", "u",
    ],
    ..BASE
};

static SHELL: Spec = Spec {
    line: &[b"#"],
    quotes: &[q(b"\"", b"\"", true, true), q(b"'", b"'", false, true)],
    keywords: &[
        "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if",
        "in", "local", "readonly", "return", "select", "then", "until", "while",
    ],
    consts: &["false", "true"],
    // `a#b` is one word: `#` starts a comment only after a blank.
    word: b"-#",
    dollar: true,
    comment_at_word: true,
    ..BASE
};

static LUA: Spec = Spec {
    line: &[b"--"],
    block: Some((b"--[[", b"]]")),
    quotes: &[q(b"[[", b"]]", false, true), DQ, SQ],
    keywords: &[
        "and", "break", "do", "else", "elseif", "end", "for", "function", "goto", "if", "in",
        "local", "not", "or", "repeat", "return", "then", "until", "while",
    ],
    consts: &["false", "nil", "true"],
    calls: true,
    ..BASE
};

static JSON: Spec = Spec {
    line: &[b"//"],
    block: Some((b"/*", b"*/")),
    quotes: &[DQ],
    consts: &["false", "null", "true"],
    keys: true,
    ..BASE
};

static CSS: Spec = Spec {
    block: Some((b"/*", b"*/")),
    quotes: &[DQ, SQ],
    keywords: &["!important", "and", "from", "not", "only", "to"],
    word: b"-",
    calls: true,
    at_attr: true,
    keys: true,
    ..BASE
};

static SQL: Spec = Spec {
    line: &[b"--"],
    block: Some((b"/*", b"*/")),
    quotes: &[q(b"'", b"'", false, false), DQ],
    keywords: &[
        "ADD",
        "ALL",
        "ALTER",
        "AND",
        "AS",
        "ASC",
        "BEGIN",
        "BETWEEN",
        "BY",
        "CASE",
        "CHECK",
        "COMMIT",
        "CONSTRAINT",
        "CREATE",
        "DEFAULT",
        "DELETE",
        "DESC",
        "DISTINCT",
        "DROP",
        "ELSE",
        "END",
        "EXISTS",
        "FOREIGN",
        "FROM",
        "GROUP",
        "HAVING",
        "IF",
        "IN",
        "INDEX",
        "INNER",
        "INSERT",
        "INTO",
        "IS",
        "JOIN",
        "KEY",
        "LEFT",
        "LIKE",
        "LIMIT",
        "NOT",
        "ON",
        "OR",
        "ORDER",
        "OUTER",
        "PRIMARY",
        "REFERENCES",
        "RIGHT",
        "ROLLBACK",
        "SELECT",
        "SET",
        "TABLE",
        "THEN",
        "TRANSACTION",
        "UNION",
        "UNIQUE",
        "UPDATE",
        "VALUES",
        "VIEW",
        "WHEN",
        "WHERE",
        "WITH",
    ],
    types: &[
        "BIGINT",
        "BLOB",
        "BOOLEAN",
        "CHAR",
        "DATE",
        "DECIMAL",
        "FLOAT",
        "INT",
        "INTEGER",
        "NUMERIC",
        "REAL",
        "TEXT",
        "TIMESTAMP",
        "VARCHAR",
    ],
    consts: &["FALSE", "NULL", "TRUE"],
    fold: true,
    calls: true,
    ..BASE
};

static MAKE: Spec = Spec {
    line: &[b"#"],
    quotes: &[DQ, SQ],
    keywords: &[
        "define", "else", "endef", "endif", "export", "ifdef", "ifeq", "ifndef", "ifneq",
        "include", "override", "unexport", "vpath",
    ],
    word: b"-.",
    dollar: true,
    targets: true,
    comment_at_word: true,
    ..BASE
};

static DOCKER: Spec = Spec {
    line: &[b"#"],
    quotes: &[DQ, SQ],
    keywords: &[
        "ADD",
        "ARG",
        "AS",
        "CMD",
        "COPY",
        "ENTRYPOINT",
        "ENV",
        "EXPOSE",
        "FROM",
        "HEALTHCHECK",
        "LABEL",
        "MAINTAINER",
        "ONBUILD",
        "RUN",
        "SHELL",
        "STOPSIGNAL",
        "USER",
        "VOLUME",
        "WORKDIR",
    ],
    fold: true,
    dollar: true,
    comment_at_word: true,
    ..BASE
};

static PLAIN: Spec = BASE;

pub(super) fn spec(lang: Lang) -> &'static Spec {
    match lang {
        Lang::Rust => &RUST,
        Lang::C => &C,
        Lang::JavaScript => &JS,
        Lang::Go => &GO,
        Lang::Python => &PYTHON,
        Lang::Shell => &SHELL,
        Lang::Lua => &LUA,
        Lang::Json => &JSON,
        Lang::Css => &CSS,
        Lang::Sql => &SQL,
        Lang::Make => &MAKE,
        Lang::Dockerfile => &DOCKER,
        _ => &PLAIN,
    }
}

fn find(list: &[&str], w: &[u8], fold: bool) -> bool {
    if fold {
        let up: Vec<u8> = w.iter().map(|c| c.to_ascii_uppercase()).collect();
        list.binary_search_by(|k| k.as_bytes().cmp(&up)).is_ok()
    } else {
        list.binary_search_by(|k| k.as_bytes().cmp(w)).is_ok()
    }
}

/// A word that names a function: called, a make target, a key before
/// `:`, or a Rust macro.
fn called(s: &Spec, line: &[u8], i: usize, j: usize) -> bool {
    let after = next_solid(line, j);
    (s.targets && i == 0 && after == Some(b':'))
        || (s.keys && after == Some(b':'))
        || (s.calls && after == Some(b'('))
        || (s.calls && s.raw && line.get(j) == Some(&b'!'))
}

fn is_word(c: u8, s: &Spec) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80 || s.word.contains(&c)
}

fn starts(line: &[u8], i: usize, pat: &[u8]) -> bool {
    line.len() >= i + pat.len() && &line[i..i + pat.len()] == pat
}

/// The next byte that is not a blank, from `i`.
fn next_solid(line: &[u8], i: usize) -> Option<u8> {
    line[i..].iter().copied().find(|c| !c.is_ascii_whitespace())
}

pub(super) fn lex(s: &Spec, state: u8, line: &[u8], o: &mut Out) -> u8 {
    let mut i = 0;
    // Carried in from the line before.
    if (1..16).contains(&state) {
        let (end, depth) = block_end(s, line, 0, state);
        o.push(end, Role::Comment);
        i = end;
        if depth != 0 {
            return depth;
        }
    } else if state >= 16 {
        let k = ((state - 16) / 16) as usize;
        let hashes = (state - 16) % 16;
        match quote_end(s, k, hashes, line, 0) {
            Some(end) => {
                o.push(end, Role::String);
                i = end;
            }
            None => {
                o.push(line.len(), Role::String);
                return state;
            }
        }
    }
    let line_start_word = |i: usize| line[..i].iter().all(|c| c.is_ascii_whitespace());
    // The word after `fn`, `def` or `function` is the function it defines.
    let mut defines = false;
    while i < line.len() {
        let c = line[i];
        if c.is_ascii_whitespace() {
            let mut j = i + 1;
            while j < line.len() && line[j].is_ascii_whitespace() {
                j += 1;
            }
            o.push(j, Role::Plain);
            i = j;
            continue;
        }
        // Comments.
        if let Some((open, _)) = s.block {
            if starts(line, i, open) {
                let (end, depth) = block_end(s, line, i + open.len(), 1);
                o.push(end, Role::Comment);
                if depth != 0 {
                    return depth;
                }
                i = end;
                continue;
            }
        }
        if s.line.iter().any(|p| starts(line, i, p))
            && (!s.comment_at_word || i == 0 || line[i - 1].is_ascii_whitespace())
        {
            o.push(line.len(), Role::Comment);
            return 0;
        }
        // Preprocessor lines and attributes.
        if c == b'#' && s.hash == Hash::Preproc && line_start_word(i) {
            o.push(line.len(), Role::Attr);
            return 0;
        }
        if c == b'#'
            && s.hash == Hash::Attribute
            && (starts(line, i, b"#[") || starts(line, i, b"#!["))
        {
            let end = line[i..]
                .iter()
                .position(|&b| b == b']')
                .map_or(line.len(), |p| i + p + 1);
            o.push(end, Role::Attr);
            i = end;
            continue;
        }
        if c == b'@' && s.at_attr {
            let mut j = i + 1;
            while j < line.len() && (is_word(line[j], s) || line[j] == b'.') {
                j += 1;
            }
            o.push(j, Role::Attr);
            i = j;
            continue;
        }
        if c == b'$' && s.dollar {
            let j = dollar_end(line, i);
            o.push(j, Role::Type);
            i = j;
            continue;
        }
        // Rust raw strings: r"…", r#"…"#, br"…".
        if s.raw && (c == b'r' || (c == b'b' && line.get(i + 1) == Some(&b'r'))) {
            let r = if c == b'b' { i + 1 } else { i };
            let mut j = r + 1;
            while j < line.len() && line[j] == b'#' {
                j += 1;
            }
            let hashes = j - r - 1;
            if line.get(j) == Some(&b'"') && hashes < 16 && (i == 0 || !is_word(line[i - 1], s)) {
                match raw_end(line, j + 1, hashes) {
                    Some(end) => {
                        o.push(end, Role::String);
                        i = end;
                        continue;
                    }
                    None => {
                        o.push(line.len(), Role::String);
                        // Raw strings take the quote number after the table's.
                        return 16 + 16 * s.quotes.len() as u8 + hashes as u8;
                    }
                }
            }
        }
        // Lifetimes: 'a, unless it is a char literal 'a'.
        if c == b'\'' && s.lifetimes {
            let is_char = line.get(i + 1) == Some(&b'\\')
                || line.get(i + 2) == Some(&b'\'')
                || (line.get(i + 1).is_some_and(|&b| b >= 0x80)
                    && line[i + 1..].iter().take(5).skip(1).any(|&b| b == b'\''));
            if !is_char {
                let mut j = i + 1;
                while j < line.len() && is_word(line[j], s) {
                    j += 1;
                }
                o.push(j, Role::Type);
                i = j;
                continue;
            }
            let end = char_end(line, i + 1);
            o.push(end, Role::String);
            i = end;
            continue;
        }
        // Strings.
        if let Some(k) = s.quotes.iter().position(|qt| starts(line, i, qt.open)) {
            let qt = &s.quotes[k];
            match quote_end(s, k, 0, line, i + qt.open.len()) {
                Some(end) => {
                    o.push(end, Role::String);
                    i = end;
                    if s.keys && next_solid(line, i) == Some(b':') {
                        o.recolor_last(Role::Func);
                    }
                    continue;
                }
                None if qt.multiline => {
                    o.push(line.len(), Role::String);
                    return 16 + 16 * k as u8;
                }
                None => {
                    o.push(line.len(), Role::String);
                    return 0;
                }
            }
        }
        // Numbers.
        if c.is_ascii_digit() || (c == b'.' && line.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let mut j = i + 1;
            while j < line.len() {
                let d = line[j];
                let exp = matches!(line[j - 1], b'e' | b'E') && matches!(d, b'+' | b'-');
                if d.is_ascii_alphanumeric() || d == b'_' || d == b'.' || exp {
                    j += 1;
                } else {
                    break;
                }
            }
            o.push(j, Role::Number);
            i = j;
            continue;
        }
        // Words.
        if is_word(c, s) || (c == b'!' && s.keywords.first() == Some(&"!important")) {
            let mut j = i + 1;
            while j < line.len() && is_word(line[j], s) {
                j += 1;
            }
            let w = &line[i..j];
            // A string prefix: f"…", rb'…'.
            if !s.prefixes.is_empty()
                && line.get(j).is_some_and(|&b| b == b'"' || b == b'\'')
                && s.prefixes.iter().any(|p| p.as_bytes() == w)
            {
                o.push(j, Role::String);
                i = j;
                continue;
            }
            let was_def = std::mem::take(&mut defines);
            let role = if find(s.keywords, w, s.fold) {
                defines = matches!(w, b"fn" | b"def" | b"function" | b"func");
                Role::Keyword
            } else if was_def {
                Role::Func
            } else if find(s.types, w, s.fold) {
                Role::Type
            } else if find(s.consts, w, s.fold) {
                Role::Number
            } else if called(s, line, i, j) {
                Role::Func
            } else if s.caps_types && w[0].is_ascii_uppercase() {
                Role::Type
            } else {
                Role::Plain
            };
            o.push(j, role);
            i = j;
            continue;
        }
        defines = false;
        o.push(i + 1, Role::Punct);
        i += 1;
    }
    0
}

/// The word lists, for the test that they are sorted.
#[cfg(test)]
pub(super) fn lists(s: &Spec) -> [&'static [&'static str]; 3] {
    [s.keywords, s.types, s.consts]
}

/// Where a block comment that is `depth` deep at `from` closes, and the
/// depth left at the end (0 when it closed).
fn block_end(s: &Spec, line: &[u8], from: usize, depth: u8) -> (usize, u8) {
    let Some((open, close)) = s.block else {
        return (line.len(), 0);
    };
    let mut depth = depth;
    let mut i = from;
    while i < line.len() {
        if starts(line, i, close) {
            depth -= 1;
            i += close.len();
            if depth == 0 {
                return (i, 0);
            }
        } else if s.nested && starts(line, i, open) {
            depth = (depth + 1).min(15);
            i += open.len();
        } else {
            i += 1;
        }
    }
    (line.len(), depth)
}

/// Where quote `k` closes, searching from `from`, or None if the line
/// ends first. `hashes` is for a Rust raw string carried in.
fn quote_end(s: &Spec, k: usize, hashes: u8, line: &[u8], from: usize) -> Option<usize> {
    let Some(qt) = s.quotes.get(k) else {
        return raw_end(line, from, hashes as usize);
    };
    let mut i = from;
    while i < line.len() {
        if qt.escape && line[i] == b'\\' {
            i += 2;
            continue;
        }
        if starts(line, i, qt.close) {
            return Some(i + qt.close.len());
        }
        i += 1;
    }
    None
}

/// The end of a raw string: `"` then `hashes` `#`.
fn raw_end(line: &[u8], from: usize, hashes: usize) -> Option<usize> {
    let mut i = from;
    while i < line.len() {
        if line[i] == b'"'
            && line[i + 1..]
                .iter()
                .take(hashes)
                .filter(|&&b| b == b'#')
                .count()
                == hashes
        {
            return Some(i + 1 + hashes);
        }
        i += 1;
    }
    None
}

fn char_end(line: &[u8], from: usize) -> usize {
    let mut i = from;
    while i < line.len() {
        match line[i] {
            b'\\' => i += 2,
            b'\'' => return i + 1,
            _ => i += 1,
        }
    }
    line.len()
}

/// `$name`, `${…}`, `$(…)`, `$1`, `$@`.
fn dollar_end(line: &[u8], i: usize) -> usize {
    let Some(&n) = line.get(i + 1) else {
        return i + 1;
    };
    match n {
        b'{' | b'(' => {
            let close = if n == b'{' { b'}' } else { b')' };
            line[i..]
                .iter()
                .position(|&b| b == close)
                .map_or(line.len(), |p| i + p + 1)
        }
        b'0'..=b'9' | b'@' | b'*' | b'#' | b'?' | b'$' | b'!' | b'-' => i + 2,
        _ => {
            let mut j = i + 1;
            while j < line.len() && (line[j].is_ascii_alphanumeric() || line[j] == b'_') {
                j += 1;
            }
            j.max(i + 1)
        }
    }
}
