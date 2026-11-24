# Media

`gh0st-media` shows pictures and plays video and sound. It is an SDK app:
the screen, the keys and the files. A helper it starts, `gh0st-mediad`,
decodes every file. A file that crashes a decoder takes the helper with it,
never the app, and the app starts the helper again.

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
| | HEVC, VP8, VP9, AV1, MPEG-2, MPEG-4, Theora, VC-1 | software, four threads |
| Sound | MP3, AAC and M4A, ALAC, Opus, Vorbis, FLAC, WAV, AIFF, WMA, APE, WavPack | libavcodec, through PipeWire |
| Subtitles | SRT, ASS, WebVTT and MP4 text, in the file or a `.srt` beside it; DVD, DVB and PGS pictures | libavcodec, drawn by libass |

Why the three extra libraries:

- **libheif.** FFmpeg reads a HEIC grid as separate tiles and leaves them
  to the caller, and it applies none of HEIF's rotation, mirror or crop.
  An iPhone photo is a grid of 512x512 tiles.
- **LibRaw.** FFmpeg reads DNG as TIFF and gives the raw Bayer mosaic.
  LibRaw finds the JPEG preview every RAW carries, which shows at once.
- **libwebpdemux.** FFmpeg 7.1 skips the frames of an animated WebP.

## The helper

`crates/gh0st-media/mediad` is C++17 with a plain Makefile. It shares the
JPEG reader, the player, the link and the ring with the camera helper:
those are in `native/av`. The app starts it as an SDK sidecar
(`gh0st_app::sidecar`) with its link in `GH0ST_MEDIAD_FD` and a frame ring
of four screen-sized slots in `GH0ST_MEDIAD_RING`. Slots 0 to 2 take a
playing video's pictures in turn; slot 3 answers asks.

### Messages

One message per packet, fields split by tabs, at most 512 bytes. Every ask
carries an id, the answer names it, and a path is always the last field.

| App to helper | Helper to app |
|---|---|
| `hello` | `hello 1` |
| `probe ID PATH` | `info ID KIND W H DUR_MS AUDIO SUBS FORMAT VCODEC ACODEC` |
| `tags ID PATH` | `tags ID TITLE ARTIST ALBUM TRACK DATE` |
| `thumb ID W H PATH` | `thumb ID SLOT SEQ W H`: the middle, cut to fill W x H |
| `view ID W H ZOOM CX CY PATH` | `view ID SLOT SEQ W H`: fitted in W x H |
| `forget` | |
| `quit` | |

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
- A song's picture is its embedded cover, else `cover.jpg`, `folder.jpg`
  or `front.jpg` beside it. A video's is its frame a tenth of the way in.
- Tiles are kept in `~/.cache/gh0st-media/thumbs`, one raw RGB565 file each,
  named by the path, the file's time and size, and the tile's size. A file
  that failed is kept as an empty one, so it is not decoded again. Past
  32 MB the oldest go when the helper starts.

## Limits

- A picture other than JPEG is refused past 24 MP, before it is decoded:
  a 40000x40000 PNG packs into a few kilobytes and would take 4.8 GB. The
  size comes from the file's header before a decoder sees it.
- A JPEG decodes at the size it is shown, so it may be up to 256 MP. A
  progressive one past what 96 MB holds is refused by libjpeg.
- Each still has 10 seconds. A decoder that hangs ends the helper, and the
  app starts it again and marks the file. `RLIMIT_AS` holds the helper to
  1536 MB of address space; `GH0ST_MEDIAD_AS_MB` changes it, 0 turns it off.
