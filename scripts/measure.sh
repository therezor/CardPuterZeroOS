#!/usr/bin/env bash
# Measures a launcher on the device: memory (PSS), wakeups and CPU while idle.
# Run the same way for stock and for gh0st, so the numbers compare. Usage:
#   GH0ST_HOST=pi@cardputer.local scripts/measure.sh M5CardputerZero-APPLaunch
#   GH0ST_HOST=pi@cardputer.local scripts/measure.sh gh0st
#   GH0ST_HOST=pi@cardputer.local scripts/measure.sh system     # the whole OS
set -euo pipefail

host=${GH0ST_HOST:?set GH0ST_HOST}
name=${1:?process name, for example gh0st or M5CardputerZero-APPLaunch, or system}
secs=${2:-60}

if [ "$name" = system ]; then
    # Read only: nothing here changes the system being measured. Root reads
    # every process's PSS; both dev setups have passwordless sudo.
    ssh "$host" sudo -n bash -s -- "$secs" <<'REMOTE'
secs=$1
irq() { awk '/dwc2|mmc|spi/ {for (i = 2; i <= 5; i++) s += $i} END {print s}' /proc/interrupts; }
ctx() { awk '/^ctxt/ {print $2}' /proc/stat; }
echo "== os";        . /etc/os-release; echo "$PRETTY_NAME, $(uname -r)"
echo "== memory";    grep -E 'MemTotal|MemAvailable|Cma(Total|Free)|SwapTotal|SwapFree' /proc/meminfo
echo "== gpu";       vcgencmd get_mem gpu 2>/dev/null || echo "vcgencmd missing"
echo "== packages";  echo "$(dpkg-query -W | wc -l) packages, /usr $(du -sh /usr 2>/dev/null | cut -f1)"
echo "== top pss";   for p in /proc/[0-9]*; do
                         k=$(awk '/^Pss:/ {print $2}' "$p/smaps_rollup" 2>/dev/null) || continue
                         [ -n "$k" ] && echo "$k $(cat "$p/comm")"
                     done | sort -rn | head -15
echo "== services";  systemctl list-units --type=service --state=running --no-legend --plain | awk '{print $1}' | tr '\n' ' '; echo
echo "== timers";    systemctl list-timers --no-legend --plain | awk '{print $(NF-1)}' | tr '\n' ' '; echo
echo "== listening"; ss -tulnH 2>/dev/null || echo "ss missing"
echo "== boot";      systemd-analyze 2>/dev/null | head -1
i0=$(irq) c0=$(ctx)
sleep "$secs"
i1=$(irq) c1=$(ctx)
echo "== idle ${secs}s"
echo "irqs (dwc2, mmc, spi): $((i1 - i0)), context switches: $((c1 - c0))"
for f in /sys/class/power_supply/*/current_now; do
    [ -r "$f" ] && echo "current: $(( $(cat "$f") / 1000 )) mA ($f)"
done
REMOTE
    exit 0
fi

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
