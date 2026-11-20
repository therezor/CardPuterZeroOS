#!/usr/bin/env bash
# Measures a launcher on the device: memory (PSS), wakeups and CPU while idle.
# Run the same way for stock and for zer0, so the numbers compare. Usage:
#   ZER0_HOST=pi@cardputer.local scripts/measure.sh M5CardputerZero-APPLaunch
#   ZER0_HOST=pi@cardputer.local scripts/measure.sh zer0
set -euo pipefail

host=${ZER0_HOST:?set ZER0_HOST}
name=${1:?process name, for example zer0 or M5CardputerZero-APPLaunch}
secs=${2:-60}

ssh "$host" bash -s -- "$name" "$secs" <<'REMOTE'
set -euo pipefail
name=$1 secs=$2
pid=$(pgrep -o -f "/$name( |$)" || pgrep -o -f "$name")
ticks=$(getconf CLK_TCK)
cpu() { awk '{print $14 + $15}' "/proc/$pid/stat"; }
# Every thread's switches: /proc/PID/status counts the main thread only.
switches() { cat /proc/"$pid"/task/*/status | awk '/ctxt_switches/ {s += $2} END {print s}'; }
threads() { awk '/^Threads/ {print $2}' "/proc/$pid/status"; }
echo "process   $name (pid $pid, $(threads) threads)"
grep -E '^(Rss|Pss|Private_Dirty)' "/proc/$pid/smaps_rollup" | sed 's/^/memory    /'
c0=$(cpu) s0=$(switches)
sleep "$secs"
c1=$(cpu) s1=$(switches)
echo "idle      ${secs}s"
echo "wakeups   $((s1 - s0)) context switches"
echo "cpu       $(( (c1 - c0) * 1000 / ticks )) ms"
REMOTE
