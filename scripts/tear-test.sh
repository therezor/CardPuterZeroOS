#!/usr/bin/env bash
# Full-screen tearing test for the panel. Scrolls horizontal stripes down
# through /dev/fb_lcd, one frame per refresh, using the driver's present
# ioctl. A tear shows as a vertical seam where the stripes step. Puts the
# old picture back at the end and prints how the driver timed its sends.
#   overlay   lays an opaque 320x18 banner on top through the overlay ioctl
#   overlay2  adds a masked 300x20 toast at the bottom too
# Usage:
#   GH0ST_HOST=pi@cardputer.local scripts/tear-test.sh [seconds] [px per frame] [overlay|overlay2]
set -euo pipefail

host=${GH0ST_HOST:?set GH0ST_HOST}
secs=${1:-10}
step=${2:-4}
mode=${3:-}

ssh "$host" python3 - "$secs" "$step" "$mode" <<'PY'
import ctypes, fcntl, mmap, os, struct, sys, time

PRESENT = 0x46C0  # GH0ST_FBIO_PRESENT
OVERLAY = 0x402046C1  # GH0ST_FBIO_OVERLAY
STATS = "/sys/bus/spi/devices/spi0.0/te_stats"
secs, step = float(sys.argv[1]), int(sys.argv[2])
mode = sys.argv[3] if len(sys.argv) > 3 else ""
w, h, stride = 320, 170, 640
fd = os.open("/dev/fb_lcd", os.O_RDWR)
m = mmap.mmap(fd, stride * h)
saved = m[:]
white, black = b"\xff\xff" * w, b"\x00\x00" * w
keep = []


def overlay(slot, x, y, ow, oh, colour, masked):
    px = (ctypes.c_uint16 * (ow * oh))(*([colour] * (ow * oh)))
    mask = 0
    if masked:
        # Rounded corners: the four corner pixels stay the app's.
        bits = [1] * (ow * oh)
        for i in (0, ow - 1, (oh - 1) * ow, oh * ow - 1):
            bits[i] = 0
        mb = (ctypes.c_uint8 * (ow * oh))(*bits)
        keep.append(mb)
        mask = ctypes.addressof(mb)
    keep.append(px)
    arg = struct.pack("IIHHHHQQ", slot, 0, x, y, ow, oh, ctypes.addressof(px), mask)
    fcntl.ioctl(fd, OVERLAY, arg)


def clear(slot):
    fcntl.ioctl(fd, OVERLAY, struct.pack("IIHHHHQQ", slot, 0, 0, 0, 0, 0, 0, 0))


def stats():
    try:
        out = {}
        for line in open(STATS).read().split("\n"):
            kv = line.split()
            if len(kv) == 2 and kv[1].lstrip("-").isdigit():
                out[kv[0]] = int(kv[1])
        return out
    except OSError:
        return {}


before = stats()
frames, t0, y0 = 0, time.monotonic(), 0
try:
    if mode in ("overlay", "overlay2"):
        overlay(0, 0, 0, 320, 18, 0xF800, False)
    if mode == "overlay2":
        overlay(1, 10, 140, 300, 20, 0x07E0, True)
    while time.monotonic() - t0 < secs:
        m[:] = b"".join(white if ((y + y0) // 10) % 2 else black for y in range(h))
        os.fsync(fd)
        fcntl.ioctl(fd, PRESENT)
        y0 = (y0 + step) % 20
        frames += 1
finally:
    if mode:
        clear(0)
        clear(1)
    m[:] = saved
    os.fsync(fd)
took = time.monotonic() - t0
print(f"{frames} frames in {took:.1f} s = {frames / took:.1f} fps")
after = stats()
for k in ("lead", "trail", "slept", "fresh", "blind"):
    if k in after and k in before:
        d = after[k] - before[k]
        print(f"{k}: {d} ({d / max(frames, 1):.2f} a frame)")
PY
