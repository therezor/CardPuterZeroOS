# Camera

`gh0st-camera` takes photos and video with the CardputerZero's IMX219. It is
an SDK app: the screen, the keys, the settings and the files. A helper it
starts, `gh0st-camd`, owns libcamera and the encoders.

The helper (`camd/`) takes the preview, 8 MP photos with EXIF, zoom,
thumbnails, and 1080p30 and 720p30 video. The image ships it in place of
M5's Camera, and migration `0003-camera` swaps them on older devices. What
is missing is at the end.

## The camera and its ceiling

Read from M5's page and the stock app's EXIF strings, then measured with
`--probe` (`hardware-facts.md`, Camera).

| | |
|---|---|
| Sensor | Sony IMX219, 8 MP (3280x2464), 1/4", on CSI-2. Not on the Lite |
| Lens | 2.85 mm f/2.0, fixed focus (10 cm to infinity), about 77° across |
| Driver | `dtoverlay=imx219` (2 lanes), unicam, libcamera 0.7 `vc4` with the VPU's ISP |
| Modes | 3280x2464 at 21 fps, 1920x1080 crop at 47, 1640x1232 binned at 41, 640x480 at 200 |
| H.264 | `/dev/video11`, level 4.0, at most 1920 wide: 1080p30 is the ceiling |
| JPEG | `/dev/video31` stops at 1920 wide, so 8 MP stills encode in software |

What that means for the best result:

- **Photos.** The full 3280x2464 mode, high-quality denoise (`cdn_hq`),
  libjpeg-turbo at quality 95. Zoom
  crops the sensor, so 2x is 2 MP of real detail.
- **Video.** Two sizes, each the best the sensor gives at that size:
  - *1080p sharp*: the native 1920x1080 crop, pixel for pixel. It sees
    58% of the sensor's width, so the view is narrower.
  - *720p wide*: the whole sensor binned to 1640x1232, scaled to 1280x720.
  - Nothing is upscaled and called 1080p.
- **Colour.** M5 ships no tuning file, so libcamera uses `imx219.json`,
  tuned for the Pi v2 module's 3.04 mm lens. Lens shading and colour may be
  off at the edges. A tuning file of our own, loaded through
  `LIBCAMERA_RPI_TUNING_FILE`, is the largest quality step left.
- **Memory.** CMA is 64 MB. With libcamera's four raw buffers an 8 MP
  still does not fit; the helper loads `camd/vc4.yaml`, which keeps two
  (`LIBCAMERA_RPI_CONFIG_FILE`, beside the binary or in
  `/usr/share/gh0st-camera`). Zero shutter lag at
  full size does not fit, so a photo stops the preview, takes the still in
  the full mode and starts again. The IPA keeps exposure and white balance
  across that switch, so there is no settling wait.

## Two processes

```
gh0st-camera (Rust, static)              gh0st-camd (C++, glibc)
  page, keys, files, settings   <-link->   libcamera, ISP, H.264, JPEG, mux
           \                                  /
            `---- frame ring (shared memory) -'
```

- libcamera is C++ with no C API, and our apps build as static musl. A
  child process keeps both, and fits the rule that slow work runs in child
  processes whose pipes join the loop.
- The app makes a socket pair (`Link::pair`, as the shell link) and the
  frame ring, then runs the helper with both fds open:
  `GH0ST_CAMD_FD` and `GH0ST_CAMD_RING` hold their numbers.
- The helper runs in a process group of its own. The shell stops the app's
  group as soon as the app answers `pause`, and the helper must still act
  on the `rec stop` and `stop` the app sent first.
- It exits on `quit`, which the app sends when it closes (Esc held, the
  switcher's Close). It sets `PR_SET_PDEATHSIG` to SIGTERM and treats a
  hang-up on the link the same way. Each finishes a recording first.
- Frames drive redraws; the app runs no timers. The preview runs at 25 fps,
  every second panel refresh (`frame_ms` 40).
- `GH0ST_CAMD` overrides the helper's path. `GH0ST_CAMD=fake` runs this
  binary's fake helper (`gh0st-camera --fake-helper`), for `--sim` runs.

### Messages

One message per packet, fields split by tabs, at most 512 bytes. Unknown
words are ignored both ways.

| App to helper | Meaning |
|---|---|
| `hello 1` | The protocol version |
| `mode photo`, `mode video 1080`, `mode video 720` | What `start` streams |
| `start`, `stop` | Open the sensor and stream; close it and free its memory |
| `set zoom 2.0`, `set ev -0.333`, `set flicker 50`, `set quality 95` | Controls |
| `shoot <path> [raw]` | A still to `<path>`, a DNG beside it with `raw` |
| `rec start <path> [mic]`, `rec stop` | A recording, with sound when `mic` |
| `thumb <path> <w> <h> [zoom cx cy]` | A picture of a JPEG or an MP4, exactly w x h: fitted, or `zoom` times closer around (cx, cy), 0 to 1 across. Answered in order |
| `quit` | Finish and exit |

| Helper to app | Meaning |
|---|---|
| `hello 1 <sensor> <w> <h>` | The sensor and its full size |
| `state off`, `preview`, `busy`, `rec` | What it is doing |
| `frame <slot> <seq> <w> <h>` | A preview frame waits in the ring |
| `shot ok <path> <ms>`, `shot err <path> <why>` | The still is saved, or not |
| `rec <ms> <bytes>` | Once a second while recording |
| `recdone <path>` | The recording is closed |
| `thumb <path> <slot> <seq> <w> <h>` | The picture `thumb` asked for |
| `nothumb <path>` | It could not be made: the app's next ask can go |
| `error nocamera`, `nomem`, `busy`, `io <why>` | Why it can't go on |

### The frame ring

A memfd the app maps and the helper inherits: a 64-byte header (magic,
version, slot count, pixels per slot), then four slots of a 16-byte header
and 320x170 RGB565 pixels. Slots 0 to 2 carry the preview; slot 3 carries
thumbnails.

The writer makes a slot's sequence number odd while it writes and even when
done, then announces it. The reader copies the frame and checks the number
again; a changed number drops that frame. The helper never rewrites the slot
it announced last, so at 25 fps a drop is rare.

The helper asks the ISP for the preview at its final size (226x170 for
photos, 302x170 for video). For photos it comes as RGB565 and is copied as
it is. In video mode the ISP's second output gives YUV420 (BT.709, limited
range), which the helper converts row by row: about 4.5 ms a frame, since
camera memory is uncached and is read a row at a time.

## The screen

`cargo run -p gh0st-camera --example mockups` draws every screen below into
`mockups/camera`; `GH0ST_PALETTE=Paper` draws them in another palette.

Every control is named on the screen, as the design system asks. The
action bar lies over the foot of the picture (`Bars::over`), so the
picture keeps the whole height.

- **Photos.** The 4:3 picture, 226x170, sits between two rails 47 px wide.
  Left: zoom with its keys (`z c`), and the last shot over its button on 4.
  Right: the mode and the photos left, the light with its keys (`x f`)
  over its button on 7, and the timer and grid when on. Icons, not words.
  Nothing covers the picture.
- **Video.** The 16:9 picture is 302x170. Its marks sit on dark plates: the
  size top left, the time left top right, and while recording a red dot and
  the time in the middle. Bottom left, zoom and light once changed.
- **Buttons.** Icon chips over 4 to 8: Photos, the other mode, the shutter
  in the middle (a record dot for video, a red square while recording),
  Light, and the menu. Light opens a slider over the picture's foot; 5 and
  7 turn into − and +, and Esc closes it. While recording, Photos and the
  mode wait.
- **Photos.** The OS bars return: "Photos 2 of 12" and a grid of tiles,
  five across and three down, newest first, a video badge on videos. The
  arrows move, Enter opens one, Delete is held on 6.
- **One shot.** The whole screen, fitted. 5 and 7 zoom out and in: 2x, 4x,
  8x, then 1:1, one photo pixel per screen pixel. Zoomed, the arrows look
  around a third of the view at a time and a small map shows where; at 1x,
  `z` and `c` go to the newer and older shot. Enter shows the date, size
  and pixels. Esc goes back to the grid.
- **How pictures come.** The app asks the helper for one picture at a
  time and it answers each, so a late answer can't land in the wrong place.
  A tile comes from the photo's own EXIF thumbnail (2 ms); the full view
  decodes only the part on screen at the scale it is shown, with
  libjpeg-turbo's crop and skip (50 to 105 ms for an 8 MP photo).
- **No camera.** A Lite, or the overlay missing: a message, nothing else.

| Key | Viewfinder | Review |
|---|---|---|
| Enter, Space | Take a photo; start or stop recording | Full screen |
| 4, `p` | Photos | Back |
| 5, Tab | Photo or video | Zoom out (one shot) |
| 6 | The shutter, as Enter | Delete, held |
| 7 | Light: the slider, where 5 and 7 are − and + | Zoom in (one shot) |
| `z` `c` (Left, Right) | Zoom out, in | Move; newer, older; look around |
| `x` `f` (Down, Up) | Light (exposure), in thirds of a stop | Move; look around |
| `t`, `g` | Timer (off, 3 s, 10 s), grid | |
| 8 | Menu | Menu |
| Esc | Cancel the timer; held, exit | Back to the viewfinder |

The menu adds quality, the video size and the mains frequency (50 or
60 Hz, against flicker). Keep RAW and Sound join it once the helper does
them.

- The screen stays on while the camera is in front (`awake`), recording or
  not: framing a shot takes as long as it takes. Home, a lock or the power
  key end it.
- A pause (Fn+K, the screen going dark, a lock) stops a recording, saves it
  and closes the sensor. The return starts the preview again.
- Shooting stops with less than 64 MB free on the card.

## Files and settings

- Photos and videos go to `~/Pictures/DCIM/Camera`, stock's folder:
  `CAM_YYYYmmdd_HHMMSS.jpg` and `VID_YYYYmmdd_HHMMSS.mp4`. Two in one
  second get `_2`.
- Stills are written to a `.part` file and renamed. Video is fragmented MP4
  with a keyframe each second, so a cut battery keeps all but the last one.
- Nothing goes through `/tmp`: it is a 32 MB tmpfs.
- EXIF: make, model, date with its offset, exposure, ISO, f/2.0, 2.85 mm,
  exposure bias, zoom, orientation, and a 160x120 thumbnail.

| Key | Values |
|---|---|
| `gh0st.camera.mode` | `photo`, `video` |
| `gh0st.camera.video` | `1080`, `720` |
| `gh0st.camera.timer` | `0`, `3`, `10` |
| `gh0st.camera.grid`, `.raw`, `.mic` | true or false |
| `gh0st.camera.quality` | `fine` (95), `normal` (85) |
| `gh0st.camera.flicker` | `50`, `60` |

Zoom and exposure start at 1x and ±0 on every launch.

## The helper

`crates/gh0st-camera/camd` is C++17 with a plain Makefile. Its parts:

- `main.cpp`: the loop, the protocol, and a worker thread that encodes
  stills, so the preview returns while a JPEG is written.
- `cam_libcamera.cpp`: the sensor, and `--probe`. `cam_stub.cpp` is a
  test-pattern sensor with the same interface, for hosts.
- `venc.cpp`: the hardware H.264 encoder (V4L2 mem2mem, `/dev/video11`).
  It takes the ISP's own dmabufs, so a frame is never copied.
- `mp4.cpp`: fragmented MP4, written by hand: an empty `moov`, then a
  `moof` and `mdat` each second, from keyframe to keyframe, on a thread of
  its own with a sync each time. `make mp4cat` builds a host check that
  wraps a raw H.264 file for ffprobe.
- `rec.cpp`: a recording: the encoder and the file together.
- `still.cpp`: JPEG from raw YUV420 (libjpeg-turbo), EXIF written by hand,
  and thumbnails decoded at 1/8 scale.
- `link.cpp`, `ring.cpp`: the two halves of the wire, as in `src/`.

How it gets the most from the sensor:

- The preview asks the ISP for RGB565 at its final size. If the ISP gives
  another size or YUV420, the helper scales and converts in software.
- A photo stops the preview, takes one frame in the full mode with
  high-quality denoise and the zoom's crop, then starts the preview again.
  The frame is copied to the heap and encoded on the worker. At most two
  wait; a third press answers `shot err busy`.
- Zoom sets `ScalerCrop` in each mode's own coordinates. A 2x photo is the
  1640x1232 crop at its own pixels.
- Out of CMA answers `error nomem` for the preview and `shot err nomem`
  for a photo, which then returns to the preview.
- Video mode runs two streams all the time: YUV420 at the recording size
  for the encoder and the preview from the ISP's second output. A recording
  starts on the next frame and the view doesn't jump. 30 fps, H.264 High at
  12 Mb/s (1080p) or 8 Mb/s (720p), a keyframe each second. A frame the
  encoder has no room for is dropped and counted on stderr.
- A video's picture in Review is its first preview frame, saved as a JPEG
  in `~/.cache/gh0st-camera`, out of the photos folder.
- The sensor's clock runs 1.2x slow against libcamera's model, so every
  frame time, exposure and flicker period is scaled (`hardware-facts.md`).

Builds:

- `os/build.sh camd` builds it in the image container against the pinned
  `libcamera-dev` (`use: camera-build` in `os/lock.json`, unpacked, never
  installed) into `os/out/camd`. `os/build.sh debs` builds it into the
  `gh0st-camera` deb.
- `make -C crates/gh0st-camera/camd stub O=$PWD/target/camd` builds the
  stub on any host with libjpeg-turbo.
- `GH0ST_HOST=<user>@<device> scripts/deploy.sh camera` installs the app
  and the helper. The shell must send every linked app `pause`: an older
  shell stops the app without it, and the helper keeps the sensor
  streaming in the background.
- `/usr/libexec/gh0st-camd --probe DIR` on a device prints the modes, the
  preview configs and their fps, CMA free in each, and the time of a full
  photo and its JPEG at q95 and q85.

## Tests

- `cargo test -p gh0st-camera`: the protocol both ways, the ring, file
  names, and the app over the fake helper (`tests/flow.rs`), on a thread
  and as a process. A press saves a file; the timer waits; a pause ends a
  recording and leaves no wakeups; a held 6 deletes.
- `cargo run -p gh0st-camera --example mockups` writes every screen to
  `mockups/camera`. As a test it lints each one in every palette.
- `make -C crates/gh0st-camera/camd check O=$PWD/target/camd` runs host
  checks of what reads photos from elsewhere: a crafted EXIF thumbnail
  offset and a picture past 32 MP.
- With `GH0ST_CAMD_TEST` naming a helper build, `tests/flow.rs` drives the
  app over it: a photo with EXIF, no `.part` left, a pause that wakes for
  nothing, and a shot after the return. Without it that test is skipped.

  ```sh
  make -C crates/gh0st-camera/camd stub O=$PWD/target/camd
  GH0ST_CAMD_TEST=$PWD/target/camd/gh0st-camd-stub cargo test -p gh0st-camera
  ```

## Missing

- **Sound.** Video is silent: the helper takes `mic` and ignores it, so the
  app hides the Sound row and marks video as muted. Sound means the mic to
  AAC-LC at 48 kHz, muxed by timestamp.
- **Unmeasured.** Ten minutes at 1080p30 (CPU, heat, card speed), the frame
  rate on the panel rather than `--sim`, and lens shading with the stock
  tuning.
- **Not built.** DNG (the helper ignores `raw`; the app hides Keep RAW), a level from the BMI270 and EXIF orientation, shutter,
  ISO and white balance by hand, and a tuning file for this lens.
