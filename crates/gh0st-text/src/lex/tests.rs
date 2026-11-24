use super::*;
use crate::buf::Buffer;

/// Lexes `lines` in order, carrying the state, and returns each line's
/// spans.
fn run(lang: Lang, lines: &[&str]) -> Vec<Vec<Span>> {
    let mut state = 0;
    let mut out = Vec::new();
    let mut all = Vec::new();
    for l in lines {
        state = lex(lang, state, l.as_bytes(), &mut out);
        all.push(out.clone());
    }
    all
}

/// The role of the first `word` on line `n` of `lines`; the word must sit
/// inside one span.
fn role(lang: Lang, lines: &[&str], n: usize, word: &str) -> Role {
    let spans = run(lang, lines);
    let line = lines[n];
    let at = line
        .find(word)
        .unwrap_or_else(|| panic!("{word} not in {line}")) as u32;
    let end = at + word.len() as u32;
    let mut start = 0;
    for s in &spans[n] {
        if at >= start && at < s.end {
            assert!(
                end <= s.end,
                "{word:?} on {line:?} crosses spans: {:?}",
                spans[n]
            );
            return s.role;
        }
        start = s.end;
    }
    panic!("{word:?} past the spans of {line:?}: {:?}", spans[n]);
}

fn is(lang: Lang, line: &str, word: &str, want: Role) {
    assert_eq!(role(lang, &[line], 0, word), want, "{word:?} in {line:?}");
}

#[test]
fn spans_cover_each_line_in_order() {
    let samples = [
        "fn main() { let x = r#\"a\"# + 'b'; } // done",
        "<a href=\"x\">&amp; text</a>",
        "key = \"value\" # note",
        "  - name: &anchor value",
        "@@ -1,3 +1,4 @@",
        "# Heading with `code` and [link](url)",
        "SELECT * FROM t WHERE a = 'b'; -- c",
        "",
        "\u{1F600} \u{00e9}t\u{00e9} \"\u{4e2d}\"",
    ];
    for lang in Lang::ALL {
        for line in samples {
            let mut out = Vec::new();
            lex(lang, 0, line.as_bytes(), &mut out);
            let mut last = 0;
            for s in &out {
                assert!(s.end > last, "{lang:?} {line:?}: {out:?}");
                last = s.end;
            }
            assert_eq!(last as usize, line.len(), "{lang:?} {line:?}: {out:?}");
        }
    }
}

#[test]
fn every_word_list_is_sorted_for_its_search() {
    for lang in Lang::ALL {
        let s = table::spec(lang);
        for list in table::lists(s) {
            for w in list.windows(2) {
                assert!(
                    w[0].as_bytes() < w[1].as_bytes(),
                    "{lang:?}: {} {}",
                    w[0],
                    w[1]
                );
            }
        }
    }
}

#[test]
fn rust() {
    let l = "pub fn read<'a>(x: &'a str) -> Option<u32> { println!(\"hi\"); None }";
    is(Lang::Rust, l, "pub", Role::Keyword);
    is(Lang::Rust, l, "read", Role::Func);
    is(Lang::Rust, l, "'a", Role::Type);
    is(Lang::Rust, l, "Option", Role::Type);
    is(Lang::Rust, l, "u32", Role::Type);
    is(Lang::Rust, l, "println", Role::Func);
    is(Lang::Rust, l, "\"hi\"", Role::String);
    is(Lang::Rust, l, "None", Role::Number);
    is(
        Lang::Rust,
        "let c = 'x'; let n = 0x1F + 1e-3;",
        "'x'",
        Role::String,
    );
    is(Lang::Rust, "let n = 0x1F + 1e-3;", "1e-3", Role::Number);
    is(
        Lang::Rust,
        "#[derive(Debug)] struct A;",
        "#[derive(Debug)]",
        Role::Attr,
    );
    // A raw string and a nested block comment run over lines.
    let lines = [
        "let s = r#\"one",
        "two \" still\"# ;",
        "/* a /* b */",
        "still */ fn x() {}",
    ];
    assert_eq!(role(Lang::Rust, &lines, 1, "still"), Role::String);
    assert_eq!(role(Lang::Rust, &lines, 1, ";"), Role::Punct);
    assert_eq!(role(Lang::Rust, &lines, 3, "still"), Role::Comment);
    assert_eq!(role(Lang::Rust, &lines, 3, "fn"), Role::Keyword);
}

#[test]
fn c_and_go_and_js() {
    is(Lang::C, "#include <stdio.h>", "stdio", Role::Attr);
    is(Lang::C, "  #define N 3", "N", Role::Attr);
    is(Lang::C, "int x = 1; // one", "// one", Role::Comment);
    is(Lang::C, "int x = 1;", "int", Role::Type);
    let go = ["s := `raw", "still` + x"];
    assert_eq!(role(Lang::Go, &go, 1, "still"), Role::String);
    assert_eq!(role(Lang::Go, &go, 1, "x"), Role::Plain);
    is(Lang::Go, "func main() {}", "func", Role::Keyword);
    let js = ["const t = `a ${b}", "c` ; await f()"];
    assert_eq!(role(Lang::JavaScript, &js, 1, "c`"), Role::String);
    assert_eq!(role(Lang::JavaScript, &js, 1, "await"), Role::Keyword);
    assert_eq!(role(Lang::JavaScript, &js, 1, "f"), Role::Func);
    is(
        Lang::JavaScript,
        "x === undefined",
        "undefined",
        Role::Number,
    );
}

#[test]
fn python_and_shell_and_lua() {
    let py = [
        "@cache",
        "def area(x):",
        "    s = f\"{x}\" + '''one",
        "two''' # c",
    ];
    assert_eq!(role(Lang::Python, &py, 0, "@cache"), Role::Attr);
    assert_eq!(role(Lang::Python, &py, 1, "def"), Role::Keyword);
    assert_eq!(role(Lang::Python, &py, 1, "area"), Role::Func);
    assert_eq!(role(Lang::Python, &py, 2, "f\""), Role::String);
    assert_eq!(role(Lang::Python, &py, 3, "two"), Role::String);
    assert_eq!(role(Lang::Python, &py, 3, "# c"), Role::Comment);
    is(
        Lang::Shell,
        "echo a#b $HOME ${X:-y} # real",
        "a#b",
        Role::Plain,
    );
    is(
        Lang::Shell,
        "echo a#b $HOME ${X:-y} # real",
        "$HOME",
        Role::Type,
    );
    is(
        Lang::Shell,
        "echo a#b $HOME ${X:-y} # real",
        "${X:-y}",
        Role::Type,
    );
    is(
        Lang::Shell,
        "echo a#b $HOME ${X:-y} # real",
        "# real",
        Role::Comment,
    );
    is(Lang::Shell, "if [ -f x ]; then", "then", Role::Keyword);
    let sh = ["echo 'one", "two' done"];
    assert_eq!(role(Lang::Shell, &sh, 1, "two'"), Role::String);
    assert_eq!(role(Lang::Shell, &sh, 1, "done"), Role::Keyword);
    let lua = ["--[[ a", "b ]] local x = [[s", "t]] -- c"];
    assert_eq!(role(Lang::Lua, &lua, 1, "b"), Role::Comment);
    assert_eq!(role(Lang::Lua, &lua, 1, "local"), Role::Keyword);
    assert_eq!(role(Lang::Lua, &lua, 2, "t]]"), Role::String);
    assert_eq!(role(Lang::Lua, &lua, 2, "-- c"), Role::Comment);
}

#[test]
fn data_and_build_files() {
    let j = "{\"name\": \"gh0st\", \"n\": 3, \"ok\": true}";
    is(Lang::Json, j, "\"name\"", Role::Func);
    is(Lang::Json, j, "\"gh0st\"", Role::String);
    is(Lang::Json, j, "3", Role::Number);
    is(Lang::Json, j, "true", Role::Number);
    is(Lang::Css, "a { color: red; } /* c */", "color", Role::Func);
    is(Lang::Css, "@media screen {", "@media", Role::Attr);
    is(
        Lang::Css,
        "a { color: red; } /* c */",
        "/* c */",
        Role::Comment,
    );
    is(
        Lang::Sql,
        "select a from t where b = 'x' -- c",
        "select",
        Role::Keyword,
    );
    is(
        Lang::Sql,
        "select a from t where b = 'x' -- c",
        "'x'",
        Role::String,
    );
    is(Lang::Make, "all: $(OUT) # build", "all", Role::Func);
    is(Lang::Make, "all: $(OUT) # build", "$(OUT)", Role::Type);
    is(Lang::Make, "all: $(OUT) # build", "# build", Role::Comment);
    is(
        Lang::Dockerfile,
        "from debian:trixie",
        "from",
        Role::Keyword,
    );
    is(Lang::Dockerfile, "RUN echo $HOME", "$HOME", Role::Type);
}

#[test]
fn config_files() {
    is(Lang::Conf, "[Unit]", "[Unit]", Role::Heading);
    is(Lang::Conf, "Restart=on-failure", "Restart", Role::Func);
    is(Lang::Conf, "Count=3", "3", Role::Number);
    is(Lang::Conf, "# note", "# note", Role::Comment);
    is(Lang::Toml, "[package]", "[package]", Role::Heading);
    is(Lang::Toml, "name = \"gh0st\" # why", "name", Role::Func);
    is(
        Lang::Toml,
        "name = \"gh0st\" # why",
        "\"gh0st\"",
        Role::String,
    );
    is(Lang::Toml, "name = \"gh0st\" # why", "# why", Role::Comment);
    let t = ["text = \"\"\"one", "two\"\"\"", "n = 1"];
    assert_eq!(role(Lang::Toml, &t, 1, "two"), Role::String);
    assert_eq!(role(Lang::Toml, &t, 2, "1"), Role::Number);
    is(Lang::Yaml, "  - name: &a value # c", "name", Role::Func);
    is(Lang::Yaml, "  - name: &a value # c", "&a", Role::Attr);
    is(Lang::Yaml, "  - name: &a value # c", "# c", Role::Comment);
    is(Lang::Yaml, "enabled: true", "true", Role::Number);
    is(Lang::Yaml, "---", "---", Role::Heading);
}

#[test]
fn prose_and_patches_and_markup() {
    let md = [
        "# Title",
        "Some `code` and [a link](http://x).",
        "```rust",
        "let x;",
        "```",
        "**bold**",
    ];
    assert_eq!(role(Lang::Markdown, &md, 0, "Title"), Role::Heading);
    assert_eq!(role(Lang::Markdown, &md, 1, "`code`"), Role::String);
    assert_eq!(role(Lang::Markdown, &md, 1, "[a link]"), Role::Func);
    assert_eq!(role(Lang::Markdown, &md, 1, "(http://x)"), Role::String);
    assert_eq!(role(Lang::Markdown, &md, 3, "let"), Role::String);
    assert_eq!(role(Lang::Markdown, &md, 5, "**bold**"), Role::Keyword);
    is(Lang::Diff, "+added", "+added", Role::Added);
    is(Lang::Diff, "-gone", "-gone", Role::Removed);
    is(Lang::Diff, "@@ -1 +1 @@", "@@", Role::Type);
    is(Lang::Diff, "--- a/x", "--- a/x", Role::Heading);
    let h = ["<a href=\"x\" class='y'>&amp; hi</a> <!-- a", "b --> <br/>"];
    assert_eq!(role(Lang::Markup, &h, 0, "a"), Role::Keyword);
    assert_eq!(role(Lang::Markup, &h, 0, "href"), Role::Attr);
    assert_eq!(role(Lang::Markup, &h, 0, "\"x\""), Role::String);
    assert_eq!(role(Lang::Markup, &h, 0, "&amp;"), Role::Number);
    assert_eq!(role(Lang::Markup, &h, 0, "hi"), Role::Plain);
    assert_eq!(role(Lang::Markup, &h, 1, "b"), Role::Comment);
    assert_eq!(role(Lang::Markup, &h, 1, "br"), Role::Keyword);
}

#[test]
fn files_are_known_by_name_then_first_line() {
    let d = |p: &str, first: &str| detect(Path::new(p), first.as_bytes());
    assert_eq!(d("src/main.rs", ""), Lang::Rust);
    assert_eq!(d("/etc/systemd/system/gh0st.service", ""), Lang::Conf);
    assert_eq!(d("Makefile", ""), Lang::Make);
    assert_eq!(d("Cargo.lock", ""), Lang::Toml);
    assert_eq!(d("/home/pi/.bashrc", ""), Lang::Shell);
    assert_eq!(d("README.MD", ""), Lang::Markdown);
    assert_eq!(d("tool", "#!/usr/bin/env python3"), Lang::Python);
    assert_eq!(d("tool", "#!/bin/sh -e"), Lang::Shell);
    assert_eq!(d("tool", "#!/usr/bin/env -S node"), Lang::JavaScript);
    assert_eq!(d("notes.txt", "#!/bin/sh"), Lang::Plain);
    assert_eq!(d("data", "hello"), Lang::Plain);
}

#[test]
fn a_long_line_is_lexed_up_to_its_limit() {
    let line = "x".repeat(LINE_MAX * 3);
    let mut out = Vec::new();
    lex(Lang::Rust, 0, line.as_bytes(), &mut out);
    assert_eq!(out.last().map(|s| s.end as usize), Some(LINE_MAX));
}

fn code(lines: usize) -> Vec<u8> {
    let mut s = String::new();
    for i in 0..lines {
        if i % 1000 == 500 {
            s.push_str("/* a comment\n");
            s.push_str("   over lines */\n");
        } else {
            s.push_str(&format!("let x{i} = {i}; // line\n"));
        }
    }
    s.into_bytes()
}

#[test]
fn a_screen_far_down_lexes_a_bounded_stretch() {
    let mut b = Buffer::from_bytes(&code(100_000));
    while !b.index_step(1 << 20).unwrap() {}
    let mut cache = Cache::new(Lang::Rust);
    let start = b.line_offset(60_000).unwrap().unwrap();
    cache.state_at(&mut b, start).unwrap();
    assert!(cache.lexed <= LOOK_BACK as u64, "{}", cache.lexed);
    // Scrolling on starts from what is known.
    let before = cache.lexed;
    let next = b.line_offset(60_020).unwrap().unwrap();
    cache.state_at(&mut b, next).unwrap();
    assert!(cache.lexed - before <= 20, "{}", cache.lexed - before);
    // An edit near the top doesn't relex the file down to the screen.
    let top = b.line_offset(10).unwrap().unwrap();
    b.insert(top, b"/* new */ ").unwrap();
    cache.edited(top);
    let before = cache.lexed;
    let start = b.line_offset(60_000).unwrap().unwrap();
    cache.state_at(&mut b, start).unwrap();
    assert!(
        cache.lexed - before <= LOOK_BACK as u64,
        "{}",
        cache.lexed - before
    );
}

#[test]
fn a_comment_opened_above_the_screen_colours_it() {
    let mut text = String::new();
    for i in 0..300 {
        text.push_str(&format!("let a{i} = 1;\n"));
    }
    text.push_str("/* opens here\n");
    for _ in 0..100 {
        text.push_str("inside\n");
    }
    text.push_str("*/ fn after() {}\n");
    let mut b = Buffer::from_bytes(text.as_bytes());
    while !b.index_step(1 << 20).unwrap() {}
    let mut cache = Cache::new(Lang::Rust);
    let inside = b.line_offset(350).unwrap().unwrap();
    assert_eq!(cache.state_at(&mut b, inside).unwrap(), 1);
    let after = b.line_offset(401).unwrap().unwrap();
    assert_eq!(cache.state_at(&mut b, after).unwrap(), 1);
    let past = b.line_offset(402).unwrap().unwrap();
    assert_eq!(cache.state_at(&mut b, past).unwrap(), 0);
}
