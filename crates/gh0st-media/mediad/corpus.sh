#!/bin/sh
# Writes the media corpus that mediad's checks and the device bench read.
# It installs ffmpeg and the image encoders first, so run it in a throwaway
# build container, never on a device:
#
#   docker run --rm -v "$PWD:/src:ro" -v /tmp/corpus:/out gh0st-build:TAG \
#       /src/crates/gh0st-media/mediad/corpus.sh /out [bench]
#
# Small files by default. `bench` adds 10 s clips at 480p, 720p and 1080p
# for the device's playback table (docs/media.md).
set -eu
out=${1:?out dir}
mode=${2:-}
mkdir -p "$out"
cd "$out"

if ! command -v ffmpeg >/dev/null || ! command -v heif-enc >/dev/null; then
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends ffmpeg libheif-examples \
        libheif-plugin-x265 libavif-bin libjxl-tools python3-pil >/dev/null
fi

export SVT_LOG=1
ff() { ffmpeg -hide_banner -loglevel error -y "$@"; }
src="-f lavfi -i testsrc2=size=320x240:rate=30:duration=2 -f lavfi -i sine=frequency=440:duration=2"

# Pictures. One picture drawn once, then written in every format.
ff -f lavfi -i testsrc2=size=1600x1200:duration=1 -frames:v 1 base.png
ff -i base.png -q:v 3 photo.jpg
ff -i base.png -q:v 3 -bitexact photo-plain.jpg
python3 - <<'PY'
from PIL import Image
im = Image.open("base.png").convert("RGB")
# EXIF orientation 1 to 8, as phones write them.
for o in range(1, 9):
    exif = Image.Exif()
    exif[0x0112] = o
    im.save(f"orient-{o}.jpg", quality=85, exif=exif)
im.save("progressive.jpg", quality=85, progressive=True)
im.save("photo.bmp")
im.save("photo.tiff", compression="tiff_lzw")
im.save("photo.tga")
im.save("photo.ppm")
im.convert("L").save("photo.pgm")
im.save("photo.webp", quality=80)
im.save("photo.jp2")
im.resize((256, 256)).save("icon.ico")
rgba = im.convert("RGBA")
rgba.putalpha(128)
rgba.save("alpha.png")
im.convert("I;16").save("deep.png")
frames = [im.resize((320, 240)).rotate(a) for a in range(0, 360, 30)]
frames[0].save("anim.gif", save_all=True, append_images=frames[1:], duration=80, loop=0)
frames[0].save("anim.webp", save_all=True, append_images=frames[1:], duration=80, loop=0)
frames[0].save("anim.png", save_all=True, append_images=frames[1:], duration=80, loop=0)
# A panorama and a tall strip.
Image.new("RGB", (30000, 800), (40, 90, 160)).save("pano.jpg", quality=80)
Image.new("RGB", (30000, 800), (40, 160, 90)).save("pano.png")
Image.new("RGB", (600, 9000), (160, 90, 40)).save("tall.png")
# A decompression bomb: 40000 x 40000 of one colour packs small.
Image.new("L", (40000, 40000), 0).save("bomb.png", optimize=True)
PY
heif-enc -q 60 -o photo.heic base.png >/dev/null
heif-enc -q 60 --cut-tiles 512 -o grid.heic base.png >/dev/null
avifenc -q 60 base.png photo.avif >/dev/null
cjxl -q 80 base.png photo.jxl >/dev/null 2>&1
cat > photo.svg <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="0 0 400 300">
<rect width="400" height="300" fill="#246"/><circle cx="200" cy="150" r="100" fill="#fc3"/>
</svg>
SVG
# Files that lie or end early.
head -c 20000 photo.jpg > truncated.jpg
head -c 3000 base.png > truncated.png
cp photo.jpg notreally.png
printf 'not a picture at all' > text.jpg

# Video, with sound.
ff $src -c:v libx264 -pix_fmt yuv420p -c:a aac h264.mp4
# Long enough for the player's checks to seek about.
ff -f lavfi -i testsrc2=size=320x240:rate=30:duration=10 -f lavfi -i sine=duration=10 \
    -c:v libx264 -pix_fmt yuv420p -c:a aac long.mp4
ff -display_rotation 90 -i h264.mp4 -c copy rotated.mp4
ff $src -c:v libx265 -tag:v hvc1 -pix_fmt yuv420p -c:a aac hevc.mp4
ff $src -c:v libvpx-vp9 -b:v 300k -c:a libopus vp9.webm
ff $src -c:v libvpx -b:v 300k -c:a libvorbis vp8.webm
ff $src -c:v libsvtav1 -preset 12 -c:a libopus av1.mkv
ff $src -c:v mpeg4 -q:v 5 -c:a libmp3lame mpeg4.avi
ff $src -c:v mpeg2video -q:v 5 -c:a mp2 mpeg2.ts
ff $src -c:v libtheora -q:v 5 -c:a libvorbis theora.ogv
ff $src -c:v libx264 -pix_fmt yuv420p -c:a aac clip.mov
ff $src -c:v libx264 -pix_fmt yuv420p -an silent.mp4
# Two sound tracks and subtitles: in the file and beside it.
cat > subs.srt <<'SRT'
1
00:00:00,200 --> 00:00:01,800
Hello from the corpus
SRT
ff $src -f lavfi -i sine=frequency=880:duration=2 -i subs.srt \
    -map 0:v -map 1:a -map 2:a -map 3:s -c:v libx264 -pix_fmt yuv420p -c:a aac -c:s ass \
    -metadata:s:a:0 language=eng -metadata:s:a:1 language=deu tracks.mkv
ff $src -c:v libx264 -pix_fmt yuv420p -c:a aac sidecar.mp4
cp subs.srt sidecar.srt
head -c 30000 h264.mp4 > truncated.mp4

# Sound alone, tagged, some with a cover.
a="-f lavfi -i sine=frequency=330:duration=3"
tags="-metadata title=Corpus -metadata artist=gh0st -metadata album=Tests -metadata track=1"
ff -i base.png -vf scale=300:300 cover.jpg
ff $a -i cover.jpg -map 0 -map 1 -c:a libmp3lame -c:v copy -disposition:v attached_pic -id3v2_version 3 $tags song.mp3
ff $a -i cover.jpg -map 0 -map 1 -c:a aac -c:v copy -disposition:v attached_pic $tags song.m4a
ff $a -c:a alac $tags lossless.m4a
ff $a -c:a libopus $tags song.opus
ff $a -c:a libvorbis $tags song.ogg
ff $a -i cover.jpg -map 0 -map 1 -c:a flac -c:v copy -disposition:v attached_pic $tags song.flac
ff $a -c:a pcm_s16le song.wav
ff $a -c:a pcm_s16be song.aiff
mkdir -p album
for n in 1 2 3; do
    ff -f lavfi -i sine=frequency=$((200 * n)):duration=2 -c:a libmp3lame \
        -metadata title="Track $n" -metadata album=Album -metadata track=$n "album/0$n.mp3"
done
cp cover.jpg album/cover.jpg

if [ "$mode" = bench ]; then
    for size in 854x480 1280x720 1920x1080; do
        b="-f lavfi -i testsrc2=size=$size:rate=30:duration=10 -f lavfi -i sine=duration=10"
        h=${size#*x}
        ff $b -c:v libx264 -pix_fmt yuv420p -c:a aac "bench-h264-$h.mp4"
        ff $b -c:v libx265 -tag:v hvc1 -pix_fmt yuv420p -c:a aac "bench-hevc-$h.mp4"
        ff $b -c:v libvpx-vp9 -b:v 2M -deadline realtime -cpu-used 8 -c:a libopus "bench-vp9-$h.webm"
        ff $b -c:v libsvtav1 -preset 12 -c:a libopus "bench-av1-$h.mkv"
    done
    python3 -c 'from PIL import Image; Image.new("RGB",(6000,4000),(9,99,199)).save("bench-24mp.png"); Image.new("RGB",(12000,8400),(199,99,9)).save("bench-100mp.jpg",quality=85)'
fi
rm -f base.png
ls | wc -l
