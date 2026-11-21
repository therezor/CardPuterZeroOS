# The terminal

`gh0st-term`, in `crates/gh0st-term`. It replaces stock APPLaunch's built-in
CLI page and runs `Terminal=true` apps (`apps.md`).

## On the screen

The grid fits the screen at the chosen cell size, or stays at a fixed size
with the screen as a window over it.

| Face | Cells | Grid, top bar on | Grid, top bar off |
|---|---|---|---|
| Terminus 12 (default) | 6x12 | 53x12 | 53x14 |
| Terminus 14 | 8x14 | 40x10 | 40x12 |
| Terminus 16 | 8x16 | 40x9 | 40x10 |
| JetBrains Mono 8 | 5x11 | 64x13 | 64x15 |
| JetBrains Mono 10 | 6x14 | 53x10 | 53x12 |
| JetBrains Mono 12 | 7x16 | 45x9 | 45x10 |
| JetBrains Mono 13 | 8x18 | 40x8 | 40x9 |

Terminus is a bitmap face and stays crisp at every size. JetBrains Mono is
anti-aliased. Box drawing, blocks, braille and powerline arrows are drawn per
cell, so lines join at any size.

- **Fit.** Programs see the screen's grid. A zoom reflows the text: lines that
  autowrapped join and wrap again at the new width.
- **Window.** Programs see a fixed grid, 80x24 by default. The screen shows
  part of it and follows the cursor. A line on an edge says more is beyond
  it. A zoom changes what shows, never the grid, so full-screen programs
  don't redraw.

## Keys

Typing goes to the program as xterm sends it: Ctrl with letters and symbols
(Ctrl+[ is Esc), Alt as an Esc prefix, modifiers on arrows, Home, End,
PgUp, PgDn, Insert, Delete and F1 to F12. The keyboard's Fn layer already
gives those: Fn+1..0 are F1 to F10, Fn+O and Fn+P F11 and F12, Fn+N End,
Fn+L and Fn+M PgUp and PgDn, Fn+B Insert, Fn+Backspace Delete.

- Fn+K is the switcher, not Home. In view mode, `k` sends Home.
- Shift+PgUp and Shift+PgDn scroll the history.
- Holding Esc for 3 s closes the terminal, as it closes any app.

**View mode** opens with Fn+H. Typing stops and keys do this:

| Key | Does |
|---|---|
| ↑ ↓, F X | Scroll |
| ← →, Z C | Pan, in window mode |
| PgUp PgDn | Page |
| g, G | Oldest line, back to the cursor |
| - = | Zoom out, in |
| m | Fit or window |
| b | Top bar on or off |
| / then n, N | Find, older, newer |
| [ ] | Previous, next shell prompt |
| v, then ↑ ↓ and y | Copy whole lines |
| p | Paste |
| t, w | New tab, close tab |
| 1 to 9, Tab | Go to a tab |
| s | Settings |
| h | All of these on one screen |
| q, Esc, Enter | Back to typing |

Prompt jumps need the shell to mark its prompt with OSC 133. For bash:

```sh
PS1='\[\e]133;A\a\]'"$PS1"
```

## Settings

View mode `s`, saved in `~/.config/cardputerzero/config.json`:

| Key | Values | Default |
|---|---|---|
| `gh0st.term.font` | `terminus`, `jetbrains` | `terminus` |
| `gh0st.term.zoom` | index into the face's sizes | 0 |
| `gh0st.term.mode` | `fit`, `window` | `fit` |
| `gh0st.term.window` | `80x24`, `100x30`, `120x40`, `132x43` | `80x24` |
| `gh0st.term.bar` | top bar | on |
| `gh0st.term.history_kb` | history for all tabs together | 1024 |
| `gh0st.term.cursor` | `block`, `underline`, `bar` | `block` |
| `gh0st.term.alt_esc` | Alt sends Esc first | on |

## What programs can do

xterm's common set: cursor moves, erase, insert and delete, scroll regions,
SGR with 256 colours and truecolor, the alternate screen, wide chars and
combining marks, line drawing, tab stops, bracketed paste, cursor shapes,
focus reports, synchronized output (mode 2026), DSR, DA and DECRQM.

- OSC 0 and 2 set the title. OSC 7 tells the terminal the working directory;
  a new tab starts there.
- OSC 52 copies to the terminal's clipboard. It never answers a read, so a
  remote host can't see what you copied.
- OSC 9 and 777 notify. The bell of a tab out of sight does too. While the
  terminal runs behind another app, the shell shows these as toasts.
- No title reports and no mouse.

## What it costs

- No timer runs while idle. With the top bar on, the clock wakes the app
  once a minute; with it off, nothing does.
- A frame redraws the grid and the flush sends only the rows that changed.
  Under a flood of output the screen updates at most every 20 ms, and the
  terminal reads at most 64 KB per loop turn, so keys stay answered.
- A cell is 8 bytes; an 80x24 screen is 15 KB. History keeps lines, not
  rows: text plus 8 bytes per colour change, about 60 bytes for a typical
  line. The budget is for all tabs together.
- Running behind, the terminal reads and parses output and draws nothing.

Not measured on the device yet: see `hardware-facts.md`.
