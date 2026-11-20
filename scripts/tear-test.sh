#!/usr/bin/env bash
# Full-screen tearing test for the panel. Scrolls horizontal stripes down
# through /dev/fb_lcd, one frame per refresh, using the driver's present
# ioctl. A tear shows as a vertical seam where the stripes step. Puts the
# old picture back at the end. Usage:
#   GH0ST_HOST=pi@cardputer.local scripts/tear-test.sh [seconds] [px per frame]
set -euo pipefail

host=${GH0ST_HOST:?set GH0ST_HOST}
secs=${1:-10}
step=${2:-4}

ssh "$host" python3 - "$secs" "$step" <<'PY'
import fcntl, mmap, os, sys, time

PRESENT = 0x46C0  # GH0ST_FBIO_PRESENT
secs, step = float(sys.argv[1]), int(sys.argv[2])
w, h, stride = 320, 170, 640
fd = os.open("/dev/fb_lcd", os.O_RDWR)
m = mmap.mmap(fd, stride * h)
saved = m[:]
white, black = b"\xff\xff" * w, b"\x00\x00" * w
frames, t0, y0 = 0, time.monotonic(), 0
try:
    while time.monotonic() - t0 < secs:
        m[:] = b"".join(white if ((y + y0) // 10) % 2 else black for y in range(h))
        os.fsync(fd)
        fcntl.ioctl(fd, PRESENT)
        y0 = (y0 + step) % 20
        frames += 1
finally:
    m[:] = saved
    os.fsync(fd)
print(f"{frames} frames in {time.monotonic() - t0:.1f} s = {frames / (time.monotonic() - t0):.1f} fps")
PY
