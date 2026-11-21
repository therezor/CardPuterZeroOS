# The terminal

`gh0st-term`, in `crates/gh0st-term`. It replaces stock APPLaunch's built-in
CLI page and runs `Terminal=true` apps (`apps.md`).

## On the screen

Two layouts; the menu's Grid row, or `m` in scroll mode, switches. The table gives what fits on the
screen at each size.

| Terminus | Cells | Fits, top bar on | Fits, top bar off |
|---|---|---|---|
| 12 (default) | 6x12 | 53x12 | 53x14 |
| 14 | 8x14 | 40x10 | 40x12 |
| 16 | 8x16 | 40x9 | 40x10 |
| 18 | 10x18 | 32x8 | 32x9 |
| 20 | 10x20 | 32x7 | 32x8 |

Terminus is a bitmap face: every pixel is on or off, so text stays sharp.
Anti-aliased faces blurred on the panel, as nothing hints them; JetBrains
Mono and Fixed were tried and dropped on 2026-11-21. Box drawing, blocks,
braille and powerline arrows and rounded caps are drawn per cell, so lines
join at any size.

Nerd Font icons are baked for the set yazi's theme uses (285 of them, list
and licences in `assets/fonts/nerd`). They are anti-aliased: at 6x12, 1-bit
icons fall apart. An icon draws across its cell and the blank after it, as
in kitty and WezTerm. With no blank after it, the middle of the icon shows
in one cell. Icons outside the set draw as boxes.

- **Magnify** (default). Programs see a fixed grid, 80x24 by default
  (Settings: Magnify grid). The screen shows part of it, follows the cursor,
  and pans. Zooming changes only the text size: nothing reflows and
  full-screen programs don't redraw. A line on an edge says more is beyond
  it.
- **Fit.** Programs see what fits at the current size. A zoom reflows the
  text: lines that autowrapped join and wrap again at the new width.

## Keys

Typing goes to the program as xterm sends it: Ctrl with letters and symbols
(Ctrl+[ is Esc), Alt as an Esc prefix, modifiers on arrows, Home, End,
PgUp, PgDn, Insert, Delete and F1 to F12. Per M5Stack's driver source the
keyboard's Fn layer gives those: Fn+1..0 are F1 to F10, Fn+O and Fn+P F11
and F12, Fn+N End, Fn+L and Fn+M PgUp and PgDn, Fn+B Insert, Fn+Backspace
Delete. Not measured on the device yet; the same source disagrees with
`hardware-facts.md` on two keys.

- Fn+K is the switcher, not Home. In scroll mode, `k` sends Home.
- **Fn+Space enters scroll mode, and leaves it.** Fn is sticky, so that is
  two taps. No program reads Fn.
- Fn+R and Fn+T make the text smaller and larger while typing.
- Shift+Fn+L (Shift+PgUp) enters scroll mode a page back. Shift+Fn+M pages
  forward.
- Holding Esc for 3 s closes the terminal, as it closes any app.

**Fn+Esc or Fn+H opens the menu.** Rows, most used first:

| Group | Row | Key |
|---|---|---|
| View | Scroll mode, a switch | Fn+Space |
| | Text size `‹ 6x12 ›` | `r` `t` |
| | Grid `‹ Magnify 80x24 ›` | `m` |
| | Find, Copy lines, Paste | `n` `v` `p` |
| Tabs | New tab, Tab `‹ 1 of 2 ›`, Close tab | `o` `Tab` `w` |
| Settings | Top bar, Font `‹ Terminus ›`, More settings | `b`, none, `s` |
| App | Keys, Close app | `h` |

Text size, Grid, Tab, Top bar and Font change in place: Z and C step them,
Enter steps forward, and the menu stays open. The other rows close it and
act. Close tab and Close app need Enter twice. A row's key picks it while
the menu is open.

**Scroll mode.** Typing stops and letters become shortcuts. The hint bar
shows the main keys, and `h` shows them all. The menu's rows work by their
keys here too:

| Key | Does |
|---|---|
| f x | Scroll (Fn+F and Fn+X too) |
| z c | Pan, in magnify |
| Fn+L Fn+M | Page |
| g G | Oldest line, back to the cursor |
| u i | Previous, next shell prompt |
| r t, m, b | Text size, grid, top bar |
| n | Find: type, then Enter or Fn+F for older, Fn+X for newer, Esc when done |
| v | Copy whole lines: f x extend, y or Enter copies |
| p | Paste |
| o w, Tab 1-9 | New tab, close tab, go to a tab |
| s | More settings |
| k | Send Home |
| h | Every key on one screen |
| Esc q, Fn+Space | Back to typing |

Prompt jumps need the shell to mark its prompt with OSC 133. For bash:

```sh
PS1='\[\e]133;A\a\]'"$PS1"
```

## Settings

The menu's More settings, or `s` in scroll mode. Saved in `~/.config/cardputerzero/config.json`:

| Key | Values | Default |
|---|---|---|
| `gh0st.term.zoom` | index into the face's sizes | 0 |
| `gh0st.term.mode` | `magnify`, `fit` | `magnify` |
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
  terminal runs behind another app, the shell shows these as toasts. Each
  starts with "Terminal:", and at most one goes out every 3 s, since the
  text may come from a remote host.
- Focus reports (mode 1004) go out when the terminal comes to the front or
  goes behind, and when the tab changes.
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
