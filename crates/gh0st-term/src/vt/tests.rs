use super::cell::{Color, attr};
use super::*;

fn vt(cols: usize, rows: usize) -> Vt {
    Vt::new(cols, rows, 1 << 20)
}

fn feed(v: &mut Vt, s: &str) {
    v.feed(s.as_bytes());
}

fn screen(v: &Vt) -> Vec<String> {
    (0..v.t.rows).map(|y| v.t.grid().text(y)).collect()
}

fn history(v: &Vt) -> Vec<String> {
    (0..v.t.history.len())
        .map(|i| v.t.history.line(i).text.to_string())
        .collect()
}

#[test]
fn text_and_newlines() {
    let mut v = vt(10, 3);
    feed(&mut v, "hello\r\nworld");
    assert_eq!(screen(&v), ["hello", "world", ""]);
    assert_eq!(v.t.cursor(), (5, 1));
}

#[test]
fn autowrap_waits_for_the_next_char() {
    let mut v = vt(4, 3);
    feed(&mut v, "abcd");
    assert_eq!(v.t.cursor(), (3, 0));
    feed(&mut v, "e");
    assert_eq!(screen(&v), ["abcd", "e", ""]);
    assert!(v.t.grid().rows[0].wrapped);
    // CR LF right at the edge makes no blank line.
    let mut v = vt(4, 3);
    feed(&mut v, "abcd\r\nx");
    assert_eq!(screen(&v), ["abcd", "x", ""]);
}

#[test]
fn rows_scroll_into_history_as_lines() {
    let mut v = vt(4, 2);
    feed(&mut v, "abcdef\r\n1\r\n2\r\n3");
    assert_eq!(screen(&v), ["2", "3"]);
    assert_eq!(history(&v), ["abcdef", "1"]);
}

#[test]
fn the_alternate_screen_keeps_out_of_history() {
    let mut v = vt(6, 2);
    feed(&mut v, "keep");
    feed(&mut v, "\x1b[?1049h");
    assert_eq!(screen(&v), ["", ""]);
    feed(&mut v, "a\r\nb\r\nc\r\nd");
    assert!(v.t.history.is_empty());
    feed(&mut v, "\x1b[?1049l");
    assert_eq!(screen(&v), ["keep", ""]);
    assert_eq!(v.t.cursor(), (4, 0));
}

#[test]
fn cursor_moves_and_erases() {
    let mut v = vt(6, 3);
    feed(&mut v, "aaaaaa\r\nbbbbbb\r\ncccccc");
    feed(&mut v, "\x1b[2;3H\x1b[K");
    assert_eq!(screen(&v), ["aaaaaa", "bb", "cccccc"]);
    feed(&mut v, "\x1b[1J");
    assert_eq!(screen(&v), ["", "", "cccccc"]);
    feed(&mut v, "\x1b[3;2H\x1b[2X");
    assert_eq!(screen(&v), ["", "", "c  ccc"]);
    feed(&mut v, "\x1b[2J");
    assert_eq!(screen(&v), ["", "", ""]);
}

#[test]
fn scroll_region() {
    let mut v = vt(4, 4);
    feed(&mut v, "top\r\n1\r\n2\r\nbot");
    feed(&mut v, "\x1b[2;3r\x1b[3;1H\nx");
    assert_eq!(screen(&v), ["top", "2", "x", "bot"]);
    assert!(v.t.history.is_empty());
    feed(&mut v, "\x1b[2;1H\x1b[L");
    assert_eq!(screen(&v), ["top", "", "2", "bot"]);
}

#[test]
fn insert_and_delete() {
    let mut v = vt(6, 1);
    feed(&mut v, "abcdef\x1b[1;2H\x1b[2@");
    assert_eq!(screen(&v), ["a  bcd"]);
    feed(&mut v, "\x1b[3P");
    assert_eq!(screen(&v), ["acd"]);
    feed(&mut v, "\x1b[4hXY\x1b[4l");
    assert_eq!(screen(&v), ["aXYcd"]);
}

#[test]
fn sgr_colours() {
    let mut v = vt(10, 1);
    feed(
        &mut v,
        "\x1b[1;31ma\x1b[38;5;196mb\x1b[38;2;255;0;0mc\x1b[48:2::0:0:255md\x1b[0me",
    );
    let c = &v.t.grid().rows[0].cells;
    assert_eq!(c[0].fg(), Color::Index(1));
    assert_eq!(c[0].attrs(), attr::BOLD);
    assert_eq!(c[1].fg(), Color::Rgb565(0xf800));
    assert_eq!(c[2].fg(), Color::Rgb565(0xf800));
    assert_eq!(c[3].bg(), Color::Rgb565(0x001f));
    assert_eq!(c[4].fg(), Color::DEFAULT);
    assert_eq!(c[4].attrs(), 0);
    feed(&mut v, "\x1b[94;101mf");
    let c = &v.t.grid().rows[0].cells;
    assert_eq!((c[5].fg(), c[5].bg()), (Color::Index(12), Color::Index(9)));
}

#[test]
fn wide_chars_take_two_cells_and_wrap_whole() {
    let mut v = vt(5, 2);
    feed(&mut v, "ab漢字");
    assert_eq!(screen(&v), ["ab漢", "字"]);
    let r = &v.t.grid().rows[0].cells;
    assert!(r[2].is_wide() && r[3].is_spacer());
    assert!(r[4].is_blank());
    // Writing over half a wide char clears the other half.
    feed(&mut v, "\x1b[1;4Hx");
    assert_eq!(screen(&v), ["ab x", "字"]);
}

#[test]
fn combining_marks_join_their_base() {
    let mut v = vt(5, 1);
    feed(&mut v, "e\u{301}x");
    let c = v.t.grid().rows[0].cells[0];
    assert!(c.is_cluster());
    assert_eq!(v.t.clusters.get(c.code()), "e\u{301}");
    assert_eq!(v.t.cursor(), (2, 0));
}

#[test]
fn device_reports() {
    let mut v = vt(10, 5);
    feed(&mut v, "\x1b[3;4H\x1b[6n\x1b[c\x1b[?2026$p");
    assert_eq!(
        String::from_utf8_lossy(&v.t.replies),
        "\x1b[3;4R\x1b[?62;22c\x1b[?2026;2$y"
    );
}

#[test]
fn tabs() {
    let mut v = vt(20, 1);
    feed(&mut v, "a\tb\tc");
    assert_eq!(screen(&v), ["a       b       c"]);
    feed(&mut v, "\x1b[Z\x1b[Zx");
    assert_eq!(screen(&v), ["a       x       c"]);
}

#[test]
fn save_and_restore_cursor() {
    let mut v = vt(10, 3);
    feed(&mut v, "\x1b[2;5H\x1b[31m\x1b7\x1b[H\x1b[0m\x1b8x");
    assert_eq!(v.t.cursor(), (5, 1));
    assert_eq!(v.t.grid().rows[1].cells[4].fg(), Color::Index(1));
}

#[test]
fn line_drawing_charset() {
    let mut v = vt(10, 1);
    feed(&mut v, "\x1b(0lqk\x1b(Bq");
    assert_eq!(screen(&v), ["┌─┐q"]);
}

#[test]
fn rep_repeats_the_last_char() {
    let mut v = vt(10, 1);
    feed(&mut v, "-\x1b[4b");
    assert_eq!(screen(&v), ["-----"]);
}

#[test]
fn title_and_cwd() {
    let mut v = vt(10, 1);
    feed(
        &mut v,
        "\x1b]2;vim notes.md\x07\x1b]7;file://host/home/a%20b\x1b\\",
    );
    assert_eq!(v.t.title, "vim notes.md");
    assert_eq!(v.t.cwd.as_deref(), Some("/home/a b"));
    assert_eq!(v.t.events, event::TITLE | event::CWD);
}

#[test]
fn shrinking_keeps_the_cursor_row() {
    let mut v = vt(6, 4);
    feed(&mut v, "1\r\n2\r\n3\r\n4");
    v.t.resize(6, 2);
    assert_eq!(screen(&v), ["3", "4"]);
    assert_eq!(history(&v), ["1", "2"]);
    assert_eq!(v.t.cursor(), (1, 1));
}

#[test]
fn modes_from_programs() {
    let mut v = vt(10, 2);
    feed(&mut v, "\x1b[?1h\x1b[?2004h\x1b[?25l\x1b[5 q");
    assert!(v.t.modes.app_cursor && v.t.modes.bracketed_paste);
    assert!(!v.t.modes.show_cursor);
    assert_eq!(v.t.shape, CursorShape::Bar);
    feed(&mut v, "\x1bc");
    assert!(!v.t.modes.app_cursor && v.t.modes.show_cursor);
}

#[test]
fn a_huge_osc_is_cut_not_kept() {
    let mut v = vt(10, 1);
    let mut s = String::from("\x1b]2;");
    s.push_str(&"x".repeat(100_000));
    s.push('\x07');
    feed(&mut v, &s);
    assert!(v.t.title.len() <= TITLE_MAX);
    feed(&mut v, "ok");
    assert_eq!(screen(&v), ["ok"]);
}

#[test]
fn a_narrower_screen_rewraps_the_prompt_line() {
    let mut v = vt(10, 3);
    feed(&mut v, "$ echo hello");
    assert_eq!(screen(&v), ["$ echo hel", "lo", ""]);
    v.t.resize(20, 3);
    assert_eq!(screen(&v), ["$ echo hello", "", ""]);
    assert_eq!(v.t.cursor(), (12, 0));
    v.t.resize(6, 3);
    assert_eq!(screen(&v), ["$ echo", " hello", ""]);
    assert_eq!(v.t.cursor(), (0, 2));
}

#[test]
fn osc_52_copies_but_never_answers_a_read() {
    let mut v = vt(10, 1);
    feed(&mut v, "\x1b]52;c;aGVsbG8gd29ybGQ=\x07");
    assert_eq!(v.t.clipboard.as_deref(), Some("hello world"));
    feed(&mut v, "\x1b]52;c;?\x07");
    assert!(v.t.replies.is_empty());
}

#[test]
fn prompt_marks_follow_their_line_into_history() {
    let mut v = vt(10, 2);
    feed(&mut v, "\x1b]133;A\x07$ ls\r\nout\r\n\x1b]133;A\x07$ ");
    assert!(v.t.history.line(0).flags & history::flag::PROMPT != 0);
    assert!(v.t.grid().rows[1].prompt);
}

#[test]
fn notifications() {
    let mut v = vt(10, 2);
    feed(&mut v, "\x1b]777;notify;make;done\x07");
    assert_eq!(v.t.notice.as_deref(), Some("make: done"));
    feed(&mut v, "\x1b]9;build ok\x07");
    assert_eq!(v.t.notice.as_deref(), Some("build ok"));
}

#[test]
fn erasing_below_forgets_wraps_and_prompts() {
    let mut v = vt(4, 3);
    feed(&mut v, "\x1b]133;A\x07abcdef\x1b[H\x1b[J");
    assert!(v.t.grid().rows.iter().all(|r| !r.wrapped && !r.prompt));
}

/// The same bytes fed whole, a byte at a time and in odd chunks must give
/// the same screen: the fast path must agree with vte at every cut.
#[test]
fn any_split_gives_the_same_screen() {
    let samples: &[&[u8]] = &[
        "plain \x1b[1;31mred\x1b[0m and \x1b[38;2;1;2;3mrgb\x1b[m".as_bytes(),
        "Привіт 世界 e\u{301} \x1b]2;títle\x1b\\after".as_bytes(),
        b"\x1b[?1049h\x1b[2;3Hx\x1b[?1049l\r\nback",
        b"abc\x1b[1\x18def\x1b]0;t\x07ghi\x1bPq#0\x1b\\jkl",
        b"bad \xff\xfe utf8 \xe2\x82 cut \xc3\xa9 ok\x7fdel",
        "\x1b(0lqk\x1b(B tabs\tand\x08back".as_bytes(),
    ];
    for &bytes in samples {
        let mut whole = vt(12, 4);
        feed_bytes(&mut whole, bytes);
        // vte driving the terminal for every byte, as before the fast path.
        let mut reference = vt(12, 4);
        let mut p = vte::Parser::<4096>::new_with_size();
        p.advance(&mut reference.t, bytes);
        assert_eq!(screen(&whole), screen(&reference), "vte on {bytes:?}");
        assert_eq!(whole.t.cursor(), reference.t.cursor(), "vte on {bytes:?}");
        for step in [1, 2, 3, 5, 7] {
            let mut cut = vt(12, 4);
            for chunk in bytes.chunks(step) {
                feed_bytes(&mut cut, chunk);
            }
            assert_eq!(screen(&cut), screen(&whole), "step {step} on {bytes:?}");
            assert_eq!(cut.t.cursor(), whole.t.cursor(), "step {step} on {bytes:?}");
            assert_eq!(cut.t.title, whole.t.title);
        }
    }
}

#[test]
fn a_cancelled_sequence_returns_to_text() {
    let mut v = vt(10, 1);
    feed(&mut v, "a\x1b[12\x18b");
    assert_eq!(screen(&v), ["ab"]);
}

fn feed_bytes(v: &mut Vt, b: &[u8]) {
    v.feed(b);
}
