# Text

A code and text editor for files of any size. It reads only what is on
screen, so a gigabyte log opens as fast as a short note. Up to four files
are open at once.

```sh
gh0st-text [FILE]
cargo run -p gh0st-text --example mockups   # writes mockups/text/*.png
cargo run --release -p gh0st-text --example corpus -- DIR   # test files
cargo run --release -p gh0st-text --example bench -- FILE   # timings
```

## Opening

- The file is read with `pread` through a 4 MB cache of 64 KB blocks,
  never mapped: a file another program cuts short reads short, it doesn't
  crash the editor.
- The first screen shows at once. A line index reads on in 8 ms slices and
  keeps a mark every 64 KB, 16 K marks for a gigabyte. Until it reaches the
  end, the top bar shows how far it has read, and the line count is a
  guess.
- For a file past 8 MB, a reader process brings it into memory at idle IO
  priority (`gh0st-text --warm`). The index, find and save take only what is
  there and try again 15 ms later, so the card's pace and its stalls never
  freeze the page. The reader dies with the editor.
- A file with a NUL in its first 64 KB opens as hex, read only.
- When memory runs short (`sdk.md`, Low memory), every open file lets its
  blocks and lexer marks go; they are read again as the screen needs them.
  Edits stay.
- With no file, the page offers Open a file, New file and the text files
  opened lately, newest first: 20 from the recent list every app shares
  (`apps.md`, Opening files). Settings > Privacy turns it off and clears
  it.
- Bytes stay as they were: text that is not UTF-8 shows as `?` in red and
  saves unchanged, CR LF stays CR LF, and a missing final newline stays
  missing.

## Editing

- The text is a piece table over the file and a buffer of what was typed.
  An edit touches a few pieces, never the file, whatever its size.
- Undo takes back a typing run, a paste or a command as one step. It keeps
  1000 steps and reaches back past a save, but not past one written in
  place.
- Enter keeps the line's indent, and adds one after `{`, `(` or `[`, and
  after `:` in Python and YAML. Between a pair it opens a line inside. A
  closer typed on a line of blanks steps back one indent. The indent is a
  tab where the line starts with one, else four spaces. Tab types four
  spaces.
- Ctrl K comments the cursor's line or the selected lines out, or back in,
  with the language's line comment.
- The bracket at the cursor and its pair are marked, within 64 KB.
- A line ending stays what it was: Enter on a CR LF line types CR LF.

## Files

- Ctrl O, or Open in the menu, asks the file chooser (Text, or All
  files). Each picked file opens beside the others, up to four. A file
  opened again shows the copy already open.
- Ctrl Tab shows the next file; Alt and 1 to 4 a given one; the top bar
  shows which, as 2/3. The menu lists them.
- Ctrl W closes the file on screen. With changes it asks first. Esc on a
  file with no changes closes it while others are open. The last one
  closes Text.
- Save as, in the menu, saves to a name the chooser gives. Once that save
  is done the file is called that; one that fails keeps the old name. A
  new file asks for a name on its first save.
- Files the shell hands over open the same way.

## Wrap

Wrap long lines, in the menu, breaks a line into rows the screen's width;
it is kept. Up and down move by rows. A line over 16 KB never wraps: it
scrolls sideways, as all lines do with wrap off.

## Unsaved edits

Each edit is kept in a journal in Text's data folder, one per file, so a
crash or a kill loses at most two seconds of typing.

- The journal is written two seconds after an edit, and the card starts
  writing it without the loop waiting. When Text goes behind, switches
  files or closes, it waits until the journal is on the card; a low
  battery's shutdown closes it that way too. Pulling the power can lose
  what the file system had not committed, a few seconds more.
- Saving, or undoing back to the file as saved, deletes it.
- Each record carries its length and a CRC. A record cut short by a crash
  is dropped, and only it.
- Opened again, the file offers to restore the edits when it is unchanged
  since. If it changed, what was typed opens as a new file, Edits to its
  name, to save somewhere. With four files open there is no room for it:
  the journal stays as it was, and the question comes back as the file
  shows again.
- Journals are readable by the user alone.
- A new file has no journal until its first save.

## Keys

Two key models; the menu's Keys row switches them. View, then edit is the
default, so a key pressed to look around never changes a file.

**View, then edit.** A file opens in View. f, x, z and c move, held to
repeat; Fn and L or M page. Chips find (5), go to a line (6) and save (7);
8 opens the menu. Enter or `i` edits, and Esc goes back to View. While
editing, the bar names Esc, Ctrl and S, and Fn and H.

**Always type.** Every key types. No action bar: every row is text. Help
(Fn and H), then 8, opens the menu.

The text size starts at 6x12, the terminal's; Fn and R or T, or the menu,
change it, and it is kept.

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
| Ctrl K | comment lines in or out |
| Ctrl O | open files |
| Ctrl Tab, Alt 1 to 4 | the next file, a given file |
| Ctrl W | close the file |
| Ctrl N | find next |
| Ctrl R | replace the match |
| Ctrl G | go to a line |
| Fn and R, T | text size |
| Esc | leave find, drop the selection, go back to View, or close |

Closing with changes asks first: Enter saves, Esc keeps editing, and 7
held discards them. Close without saving is in the menu too.

## Saving

- The text goes to a new file beside the old one, which is synced, renamed
  over the old one, and the folder synced. A crash or a full card part way
  leaves the old file whole. A big file saves in slices; meanwhile the
  text can't change, and a key that would change it says so.
- Symlinks are followed: the link stays, its target changes. The mode stays.
- A file with other hard links, or in a folder the user can't write, is
  written in place after a question, up to 64 MB.
- A file the user can't write is read only.
- Another program changing the file shows: inotify on its folder, then its
  device, inode, size and time. With no changes of ours the text reloads;
  with changes, the editor says so and keeps them. A file not on screen is
  checked as it comes back.

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

## Budgets

Measured by `examples/bench.rs` on the device, over the 1 GB log from
`examples/corpus.rs`. The panel's send comes on top of a key's time; it
is in `panel-driver.md`.

| | Budget |
|---|---|
| First screen of a 1 GB file | under 150 ms |
| An index or find slice | 8 ms at most |
| A key to a drawn frame | under 4 ms; one text row changes |
| A frame with nothing new | no heap allocation from Text |
| PSS with a 1 GB file open | under 12 MB |
| Wakes at idle once indexed | none |

## Limits

- The clipboard is the one every app shares, and takes about a megabyte:
  a bigger copy is refused, and a cut then keeps the text.
- Four files at once.
- With wrap, a wide character that would cross a row's end is not drawn.
