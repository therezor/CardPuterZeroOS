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

## Limits

- A picture other than JPEG is refused past 24 MP, before it is decoded:
  a 40000x40000 PNG packs into a few kilobytes and would take 4.8 GB.
  JPEG has no limit short of 32 MP, as it decodes at the size it is shown.
