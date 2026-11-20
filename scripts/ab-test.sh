#!/usr/bin/env bash
# Blind A/B test of a panel driver parameter, judged by eye. Each round plays
# the scrolling-stripe test twice, labelled A and B on the panel. Which one is
# "old" and which "new" is random per round. You pick the one that tears
# less, then the script reveals the order and the tally. Usage:
#   GH0ST_HOST=pi@cardputer.local scripts/ab-test.sh [rounds] [seconds]
# The default compares the driver's scan sync off (te=0, like the old driver)
# with on (te=1). Another parameter:
#   AB_PARAM=te_margin_us AB_OLD=0 AB_NEW=300 scripts/ab-test.sh
set -euo pipefail

host=${GH0ST_HOST:?set GH0ST_HOST}
rounds=${1:-3}
secs=${2:-6}
param=${AB_PARAM:-te}
old=${AB_OLD:-0}
new=${AB_NEW:-1}
step=${AB_STEP:-6}
path=/sys/module/panel_mipi_dbi_m/parameters/$param

if ! ssh -n "$host" "test -w $path || sudo -n true" 2>/dev/null; then
    echo "need passwordless sudo on $host to set $param" >&2
    exit 1
fi
orig=$(ssh -n "$host" cat "$path")
restore() { ssh -n "$host" "echo $orig | sudo tee $path >/dev/null"; }
trap restore EXIT

# Sets the parameter, shows the round's letter, then scrolls stripes.
play() { # letter value
    ssh "$host" python3 - "$1" "$2" "$path" "$secs" "$step" <<'PY'
import fcntl, mmap, os, subprocess, sys, time

letter, value, param, secs, step = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4]), int(sys.argv[5])
PRESENT = 0x46C0  # GH0ST_FBIO_PRESENT
w, h = 320, 170
subprocess.run(["sudo", "tee", param], input=value.encode(), stdout=subprocess.DEVNULL, check=True)
fd = os.open("/dev/fb_lcd", os.O_RDWR)
m = mmap.mmap(fd, w * 2 * h)
saved = m[:]

def show(rows):
    m[:] = b"".join(rows)
    os.fsync(fd)
    fcntl.ioctl(fd, PRESENT)

glyphs = {
    "A": [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
    "B": ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."],
}
s = 14
gx, gy = (w - 5 * s) // 2, (h - 7 * s) // 2
grey, white = b"\x08\x42", b"\xff\xff"
rows = []
for y in range(h):
    gr = (y - gy) // s
    line = bytearray(grey * w)
    if 0 <= gr < 7:
        for c, ch in enumerate(glyphs[letter][gr]):
            if ch == "#":
                x0 = gx + c * s
                line[x0 * 2:(x0 + s) * 2] = white * s
    rows.append(bytes(line))
try:
    show(rows)
    time.sleep(1.2)
    on, off = b"\xff\xff" * w, b"\x00\x00" * w
    t0, y0 = time.monotonic(), 0
    while time.monotonic() - t0 < secs:
        show([on if ((y + y0) // 12) % 2 else off for y in range(h)])
        y0 = (y0 + step) % 24
finally:
    m[:] = saved
    os.fsync(fd)
PY
}

declare -a order answers
for r in $(seq 1 "$rounds"); do
    if ((RANDOM % 2)); then a=$old b=$new order[r]="A=old B=new"; else a=$new b=$old order[r]="A=new B=old"; fi
    echo "Round $r of $rounds: watch the panel."
    play A "$a"
    play B "$b"
    read -r -p "Round $r: which tore less? [a/b/same] " ans
    answers[r]=${ans:-same}
done

echo
echo "$param: old=$old new=$new"
pick_new=0 pick_old=0
for r in $(seq 1 "$rounds"); do
    ans=$(printf %s "${answers[r]}" | tr "[:upper:]" "[:lower:]")
    verdict=same
    case "$ans" in
        a) [[ ${order[r]} == "A=new"* ]] && verdict=new || verdict=old ;;
        b) [[ ${order[r]} == *"B=new" ]] && verdict=new || verdict=old ;;
    esac
    [[ $verdict == new ]] && pick_new=$((pick_new + 1))
    [[ $verdict == old ]] && pick_old=$((pick_old + 1))
    echo "Round $r: ${order[r]}, you picked ${ans}: $verdict"
done
echo "new won $pick_new, old won $pick_old, same $((rounds - pick_new - pick_old))"
