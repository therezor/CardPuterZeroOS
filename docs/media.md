# Media

`gh0st-media` shows pictures and plays video and sound. It is an SDK app:
the screen, the keys and the files. A helper it starts, `gh0st-mediad`,
decodes every file. A file that crashes a decoder takes the helper with it,
never the app, and the app starts the helper again.

## The app

One page in five views. `cargo run -p gh0st-media --example mockups`
draws each into `mockups/media`.

| View | What it shows | Keys |
|---|---|---|
| Start | Camera, Pictures, Videos, Music (the XDG folders) and Home, with how much each holds, then Other folder | Up and Down, Enter |
| Grid | A folder as tiles, five across: subfolders first with their cover or newest picture, then pictures, videos and songs, newest first. A folder of songs goes by name. The top bar names the tile | the arrows, Enter opens, 6 sorts by name or date, `p` starts a slideshow |
| Viewer | A picture on the whole screen: fitted, or filling it (6), closer in steps up to its own pixels (5 and 7). Zoomed or filled, a map shows where the view is | Left and Right step through the folder; zoomed, the arrows look around. Enter shows the details |
| Player | A video, or a moving picture that loops. Its time and a line over the buttons | Enter or 6 plays and pauses, 5 and 7 or Left and Right move 10 s. The menu has speed, the sound track, subtitles, A to B and the next picture |
| Music | A song with its cover, title, artist and album; the folder is the queue | 6 plays and pauses, 5 and 7 the previous and next song. `q` the queue, `s` shuffle, `r` repeat |

- Opened with a file (`gh0st-media PATH`, as the shell starts it for one),
  it shows that file, and Left and Right step through its folder.
- The menu's Open file (`o`) and Choose folder (`g`), and the start page's
  Other folder, ask the file chooser (`portal.md`): pictures, videos or
  songs, several at once, or a folder for the grid. A file picked shows
  with its folder's others; Esc goes back to that folder.
- The music queue keeps its songs whatever folder the grid shows, so
  music plays on while you look through pictures.
- A clip of five minutes or more goes on from where it stopped, unless it
  was near its start or end: the newest 64 places, in
  `~/.local/share/gh0st-media/positions`. Only while Settings > Privacy
  lets apps remember recent files; its Clear removes them.
- Music plays on behind other apps and with the screen dark. A video and a
  slideshow pause when the app leaves the front; a video keeps the screen
  on while it plays.
- Behind another app, music has a Now playing notice with Pause (Play
  once paused) and Next as chips. It follows the song, and goes when Media
  is back in front.
- When memory runs short (`sdk.md`, Low memory), the helper drops the
  picture it keeps for pan and zoom, unless it is on screen.
- A file that fails shows why on its own screen. If the helper dies on a
  file, the app starts it again and that file is marked.
- `src/fake.rs` stands in for the helper on a host: it paints each file by
  its name, so the mockups and `tests/flow.rs` need no decoders.

## Formats

What the image's libraries decode: FFmpeg 7.1 (`libavcodec` 61,
`libavformat` 61, Debian's build with dav1d, libjxl, librsvg, libopenjpeg,
libwebp and libvpx), libjpeg-turbo, and three libraries where FFmpeg falls
short. `crates/gh0st-media/mediad/corpus.sh` writes a file of each kind;
`make check` in `mediad/` opens every one.

| Kind | Formats | Decoder |
|---|---|---|
| Photos | JPEG, baseline and progressive, any EXIF orientation | libjpeg-turbo, scaled and cropped while it decodes |
| Pictures | PNG (8 and 16 bit, alpha), WebP, BMP, TIFF, TGA, PPM, PGM, QOI, ICO, JPEG 2000, JPEG XL, SVG, PSD, EXR, HDR | libavcodec |
| Phone photos | HEIC and AVIF, tiled grids included, with their rotation and mirror | libheif, on libavcodec's HEVC and dav1d |
| Camera RAW | DNG, CR2, CR3, NEF, ARW, RAF, ORF, RW2 | LibRaw: the embedded preview, else a half-size render |
| Moving pictures | GIF, APNG | libavformat, as silent loops |
| | Animated WebP | libwebpdemux, as a silent loop |
| Video | MP4, MOV, M4V, MKV, WebM, AVI, MPEG-TS, OGV, FLV, 3GP, WMV | libavformat |
| | H.264 | the VPU (`h264_v4l2m2m`) up to 1080p, then software |
| | HEVC, VP8, VP9, AV1, MPEG-2, MPEG-4, Theora, VC-1 | software, two to four threads |
| Sound | MP3, AAC and M4A, ALAC, Opus, Vorbis, FLAC, WAV, AIFF, WMA, APE, WavPack | libavcodec, through PipeWire |
| Subtitles | SRT, ASS, WebVTT and MP4 text, in the file or a `.srt` beside it; DVD, DVB and PGS pictures | libavcodec, drawn by libass |

Why the three extra libraries:

- **libheif.** FFmpeg reads a HEIC grid as separate tiles and leaves them
  to the caller, and it applies none of HEIF's rotation, mirror or crop.
  An iPhone photo is a grid of 512x512 tiles.
- **LibRaw.** FFmpeg reads DNG as TIFF and gives the raw Bayer mosaic.
  LibRaw finds the JPEG preview every RAW carries, which shows at once.
- **libwebpdemux.** FFmpeg 7.1 skips the frames of an animated WebP.

libheif, LibRaw, libass and libavfilter load the first time a file needs
them (`native/av/lazy.h`): a HEIC, a RAW, subtitles, a speed other than
1x. Linked, Debian's libavfilter alone maps speech voices and libplacebo
into the helper. A picture or a video at normal speed costs none of it.

Opened from another app (Files, `gh0st open`), the file shows on the
whole screen, and Esc goes back to that app. Browsed to here, Esc goes
back to the folder.

Music plays on when its page closes: behind the grid, under a still
picture, and behind other apps. The top bar says Playing; Enter on the
song playing goes back to its page. Leaving for another app posts a Now
playing notice, gone when Media is in front again. A video takes the
player and stops it.

While music plays, Media is `org.mpris.MediaPlayer2.gh0st_media` on the
session bus: `playerctl`, a headset's buttons (through BlueZ's
mpris-proxy) and any player widget see the track and press Play, Pause,
Next, Previous, Stop and Seek. The name goes when the music stops. The bus
fd joins the page's loop: no thread, and nothing wakes at idle.

## The helper

`crates/gh0st-media/mediad` is C++17 with a plain Makefile. It shares the
JPEG reader, the player, the link and the ring with the camera helper:
those are in `native/av`. The app starts it as an SDK sidecar
(`gh0st_app::sidecar`) with its link in `GH0ST_MEDIAD_FD` and a frame ring
of four screen-sized slots in `GH0ST_MEDIAD_RING`. Slots 0 to 2 take a
playing video's pictures in turn; slot 3 answers asks.

### Messages

One message per packet, fields split by tabs, at most 8 KB. Every ask
carries an id, the answer names it, and a path is always the last field,
escaped as the shell link escapes one (`%XX`), so a tab or any byte in a
name survives. Messages may be 8 KB; one too long is refused, not cut.

| App to helper | Helper to app |
|---|---|
| `hello` | `hello 2` |
| `probe ID PATH` | `info ID KIND W H DUR_MS AUDIO SUBS FORMAT VCODEC ACODEC` |
| `tags ID PATH` | `tags ID TITLE ARTIST ALBUM TRACK DATE` |
| `thumb ID W H PATH` | `thumb ID SLOT SEQ W H`: the middle, cut to fill W x H |
| `view ID W H ZOOM CX CY PATH` | `view ID SLOT SEQ W H`: fitted in W x H |
| `forget` | |
| `quit` | |

Playing:

| App to helper | Helper to app |
|---|---|
| `play open ID W H FLAGS PATH` | `frame SLOT SEQ W H` for each picture, `play ID POS DUR STATE DROPPED SLOW` each second and on a change |
| `play next ID PATH` | opens the next track now; it plays the moment this one ends, and `play` then names it |
| `play pause`, `play resume`, `play stop` | |
| `play seek MS`, `play step` | a step shows the next picture while paused |
| `play speed PERCENT` | 50 to 200, the sound at its own pitch |
| `play ab A B`, `play ab off` | A to B over and over |
| `play tracks AUDIO SUBS` | opens it again where it was, with those tracks |
| | `slow ID STEP` once decoding skips more to keep up; `playerr ID WHY` |

- `FLAGS` is a comma list, or empty: `loop`, `silent`, `paused`,
  `audio=N`, `subs=N` (a `.srt` beside the file counts after the file's
  own) and `at=MS`. A GIF, an APNG or an animated WebP always loops,
  silent.
- `STATE` is `playing`, `paused` or `done`. `DROPPED` counts pictures
  dropped for being late.
- `KIND` is `image`, `anim`, `video` or `audio`. `W` and `H` are as shown,
  after the EXIF orientation or the video's rotation. A song's are its
  cover's.
- `ZOOM` 1 fits the whole picture; more is closer, up to one picture pixel
  per screen pixel. `CX` and `CY` are where the view looks, 0 to 1 across.
- Any ask can end in `fail ID WHY`: `unsupported`, `toobig`, `corrupt`, `io`
  or `nomem`.

### Pictures

- **JPEG** decodes only what the view shows: libjpeg-turbo skips the rows
  above, crops the columns and scales the DCT. A pan or a zoom is a new
  decode of a few milliseconds.
- **Everything else** decodes once into a picture of at most 8 MP, kept for
  the file on screen: a pan or a zoom reads from it and decodes nothing.
  Leaving the viewer sends `forget`, which drops it and gives the memory
  back to the system. A picture with no alpha scales straight to RGB565,
  with no full-size RGBA copy.
- A song's picture is its embedded cover, else `cover.jpg`, `folder.jpg`
  or `front.jpg` beside it. A video's is its frame a tenth of the way in.
- Tiles are kept in `~/.cache/gh0st-media/thumbs`, one raw RGB565 file each,
  named by the path, the file's time and size, and the tile's size. A file
  that failed is kept as an empty one, so it is not decoded again. Past
  32 MB the least used go, down to 24 MB, when the helper starts and as
  tiles are added. A tile shown again counts as new, once a day at most,
  so the card isn't written on every look.

### Playing

The player is the camera's (`native/av/player.cpp`), built here with
speed and subtitles. One thread reads the file, one decodes the picture
and keeps it on the clock, one decodes the sound and writes it to
PipeWire, so the loop's thread only moves finished pictures to the ring.

- H.264 decodes on the VPU (`h264_v4l2m2m`) up to 1080p; everything else in
  software. A software decoder gets 2 threads up to 640x480, 3 up to
  1280x720, 4 above: each thread holds pictures of its own. The sound is
  the clock.
- On the board H.264, HEVC, VP9 and AV1 keep 30 fps up to 720p, measured
  on a test pattern. At 1080p H.264 on the VPU takes a third of one core;
  HEVC and AV1 take over two cores and drop 5 to 8% of pictures. The
  numbers are in `hardware-facts.md`, Media.
- When decoding falls behind, a late picture is dropped. Past a quarter
  dropped in 2 s it skips more, a step at a time: the loop filter on
  frames nothing refers to, then those frames, then half resolution where
  the codec has it, then all but key frames, and says `slow` once.
- Sound alone plays with no picture thread. A video's display matrix turns
  its picture upright.
- Speed runs the sound through libavfilter's `atempo`; subtitles are drawn
  by libass onto each picture, at 1.6 times libass's size so a line stays
  readable on 170 rows. Picture subtitles (DVD, DVB, PGS) are scaled with
  the video.
- An animated WebP plays through libwebpdemux on a thread of its own. A
  frame shorter than 20 ms shows for 100 ms, as FFmpeg shows a GIF's.
- A sound track whose channels, sample format or rate change partway is
  resampled afresh at the change. The sound server's stream stays as it
  opened.
- `make playcheck`, then `playcheck --more CORPUS`, checks sound alone,
  sound that turns from stereo to mono, a 4K video refused, a turned
  video, a loop, A to B, a step, twice the speed and both kinds of
  subtitle.
- `make bench` builds `bench`, which plays or shows a file as the helper
  does and prints pictures a second, drops, CPU and peak memory.
  `bench play FILE 10 software` keeps H.264 off the VPU.

## Limits

- A picture other than JPEG is refused past 24 MP, before it is decoded:
  a 40000x40000 PNG packs into a few kilobytes and would take 4.8 GB. The
  size comes from the file's header before a decoder sees it.
- A JPEG decodes at the size it is shown, so it may be up to 256 MP. A
  progressive one past what 96 MB holds is refused by libjpeg.
- A video's tile is held to 24 MP too. Playing is held to twice 1080p
  (4.1 MP), checked before a decoder starts its threads, and every decoder
  refuses a picture past it. A 4K video shows as a tile but doesn't play.
  This limit is reasoned from the 1080p row in `hardware-facts.md`, not
  measured at 4K.
- An animated WebP is held to the same 4.1 MP, as it plays from two
  canvases of its size, and to a 32 MB file.
- A RAW with no JPEG preview is refused past 50 MP of mosaic, borders
  counted. One with a preview shows at any size.
- A `.srt` beside a video is read up to 4 MB and 20000 lines, always as
  SRT.
- Each still has 10 seconds, and so has each open of something to play. A
  decoder or demuxer that hangs ends the helper, and the app starts it
  again and marks the file.
- The helper sets its OOM score to 1000, so out of memory the kernel ends
  it before anything else. `RLIMIT_AS` holds it to 1536 MB of address
  space, which stops a decoder that asks for gigabytes at once but sits
  past the board's RAM: thread stacks, malloc's arenas and the libraries
  count too. `GH0ST_MEDIAD_AS_MB` changes it, 0 turns it off.
