# Text

A code and text editor for files of any size. It reads only what is on
screen, so a gigabyte log opens as fast as a short note.

```sh
gh0st-text FILE
cargo run -p gh0st-text --example mockups   # writes mockups/text/*.png
cargo run --release -p gh0st-text --example corpus -- DIR   # test files
cargo run --release -p gh0st-text --example bench -- FILE   # engine timings
```

## Opening

- The file is read with `pread` through a 4 MB cache of 64 KB blocks,
  never mapped: a file another program cuts short reads short, it doesn't
  crash the editor.
- The first screen shows at once. A line index reads on in 8 ms slices and
  keeps a mark every 64 KB, 16 K marks for a gigabyte. Until it reaches the
  end, the top bar shows how far it has read, and the line count is a
  guess.
- A file with a NUL in its first 64 KB opens as hex, read only.
- Bytes stay as they were: text that is not UTF-8 shows as `?` in red and
  saves unchanged, CR LF stays CR LF, and a missing final newline stays
  missing.

## Editing

- The text is a piece table over the file and a buffer of what was typed.
  An edit touches a few pieces, never the file, whatever its size.
- Undo takes back a typing run, a paste or a command as one step. It keeps
  1000 steps and reaches back past a save.
- Enter keeps the line's indent. Tab types four spaces.

## Keys

Two key models; the menu's Keys row switches them.

**Always type.** Every key types. No action bar: every row is text. Help
(Fn and H), then 8, opens the menu.

**View, then edit.** A file opens in View. f, x, z and c move, held to
repeat; Fn and L or M page. Chips find (5), go to a line (6) and save (7);
8 opens the menu. Enter or `i` edits, and Esc goes back to View. While
editing, the bar names Esc, Ctrl and S, and Fn and H.

| Keys | What |
|---|---|
| Fn and F, X, Z, C | move; with Shift, select |
| Ctrl and an arrow | a word left or right, the top or end of the file |
| Fn and L, M | page up, down |
| Ctrl A, Ctrl E or Fn N | line start (the indent, then column 0), line end |
| Ctrl S | save |
| Ctrl Z, Ctrl Y | undo, redo |
| Ctrl C, X, V | copy, cut, paste |
| Ctrl F, then Enter | find, then the next match; `/` in View |
| Ctrl N | find next |
| Ctrl R | replace the match |
| Ctrl G | go to a line |
| Fn and R, T | text size |
| Esc | leave find, drop the selection, go back to View, or close |

Closing with changes asks to save first. Close without saving is in the
menu, and asks twice.

## Saving

- The text goes to a new file beside the old one, which is synced, renamed
  over the old one, and the folder synced. A crash or a full card part way
  leaves the old file whole. A big file saves in slices.
- Symlinks are followed: the link stays, its target changes. The mode stays.
- A file with other hard links, or in a folder the user can't write, is
  written in place after a question, up to 64 MB.
- A file the user can't write is read only.
- Another program changing the file shows: inotify on its folder, then its
  device, inode, size and time. With no changes of ours the text reloads;
  with changes, the editor says so and keeps them.

## Colour

A lexer per language colours a line at a time, from the state the line
starts in: inside a block comment, a string that runs on. Only lines on
screen are lexed. The state a screen starts in comes from a checkpoint
every 256 lines, or from at most 512 lines up; past that, the screen starts
plain. A line is lexed up to 16 KB.

| Languages | By |
|---|---|
| sh, Python, Rust, C and C++, JavaScript and TypeScript, Go, Lua, JSON, CSS, SQL, Makefile, Dockerfile | one lexer driven by a table per language |
| Markdown, diffs, HTML and XML, INI and systemd units, TOML, YAML | small lexers of their own |

A file's language comes from its name, then a `#!` on its first line.
Colours are the palette's `Syntax` roles, drawn from its terminal colours,
so a scheme file colours code too.

## Limits

- One file, no tabs yet.
- The clipboard is the editor's own.
- No soft wrap yet: long lines scroll sideways.
- Save as and Open wait for the file picker.
