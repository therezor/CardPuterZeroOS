# Panel driver

**Tear-free 50 fps on the CardputerZero's SPI panel, full-screen redraws included.**

A patched ST7789V driver that times every send against the panel's scan, and a present ioctl that
locks gh0st's animations to the refresh.

```sh
cd os/kernel/panel-mipi-dbi-m && sudo ./install.sh   # on the device, then reboot
```

## Key improvements

| | M5 driver | This driver |
|---|---|---|
| Full-screen redraws | 40 fps, tearing | 50.8 fps, one frame per refresh |
| fbdev write to send | 50 ms | 6.6 ms |
| Frames of a 7-frame glide shown | 2 to 3 | 7 |
| Full-frame prep | 6.7 ms | 1.3 ms |

## How it works

### Sends timed against the scan

The panel scans its 320 lines in send order, one refresh every 19.76 ms (50.6 Hz). A send is safe
if it starts before the scan reaches its first line (lead), or late enough that the scan passes its
last line first (trail). Otherwise the driver sleeps until the trail time.

For a send to lines l1 to l2 that takes W on the wire, with blanking b, line time t and phase p
since blanking started:

- lead: p ≤ b + l1 × t − margin, and the write is faster per line than the scan
- trail: p ≥ b + (l2 + 1) × t − W + margin

A full frame can lead until 0.28 ms and trail from 2.5 ms. M5's driver doesn't use TE at all, so
its sends start anywhere in the refresh.

### TE tracking

The TE interrupt timestamps both edges while frames flow, and learns the period and blanking time
from them. It is unlazy, so a masked edge is never replayed. After 20 edges with no send it turns
itself off, so an idle panel takes no interrupts.

### Fast copy

Each damaged row is copied out of the write-combined GEM buffer with one `memcpy`, then swapped
and transposed in a single pass. M5's path loaded every pixel from uncached memory: 6.7 ms a frame.

### Present ioctl

`GH0ST_FBIO_PRESENT` (`_IO('F', 0xC0)`) returns once everything written to `/dev/fb_lcd` is on the
wire. gh0st calls it after each animation frame and draws the next one at once, so frames lock to
the refresh. M5's driver answers `ENOTTY`, and gh0st falls back to a 20 ms timer.

### Shorter fbdev delay

fbdev waits one jiffy (4 ms) after a write before it sends, not 50 ms. Stock apps get the faster
path without changes.

## Reference

### Module parameters

In `/sys/module/panel_mipi_dbi_m/parameters/`.

| Parameter | Default | Live | What it does |
|---|---|---|---|
| `te` | Y | yes | Time sends against the scan. N sends at once. |
| `te_margin_us` | 300 | yes | Margin around the scan. |
| `te_idle_edges` | 20 | yes | Edges without a send before the TE interrupt turns off. |
| `te_vblank_high` | Y | yes | TE is high during blanking. |
| `te_timeout_ms` | 50 | yes | Longest wait for a TE edge. |
| `native_order` | Y | no | Send along the panel's scan lines. |
| `madctl_native` | 0x90 | no | MADCTL with `native_order`. |
| `frctrl` | 0x15 | no | ST7789 FRCTRL2. 0x15 is 50.6 Hz. |
| `fbdev_delay` | 1 | no | Jiffies before fbdev sends. 0 keeps the kernel's 50 ms. |

### Stats

`/sys/bus/spi/devices/spi0.0/te_stats` shows `te_on`, the period and blanking time, the wire speed,
how many sends led, trailed or slept, and the phase of the last send.

### Testing

```sh
GH0ST_HOST=pi@<device> scripts/tear-test.sh 10   # scrolling stripes; a tear is a vertical seam
```

### Uninstall

```sh
sudo ./install.sh uninstall   # M5's driver back, then reboot
```
