# Panel driver

`os/kernel/panel-mipi-dbi-m` drives the CardputerZero's ST7789V panel (320x170 RGB565 on SPI0).
It is M5's copy of Noralf Trønnes's `panel-mipi-dbi` driver with gh0st's changes. This page
explains how a frame gets from an app to the glass, how the driver times each send against the
panel's scan, and how gh0st paces its animations to the panel. The numbers come from ftrace and
`GH0ST_LOG=debug` runs on the device (kernel `6.18.39+rpt-rpi-v8`, 2026-11-20). The raw facts are in
`hardware-facts.md`.

## From a write to the glass

An app draws through `/dev/fb_lcd`, the DRM fbdev emulation, in these steps:

1. The app writes its mmap of the framebuffer. The mapping is a shadow buffer in system memory.
   The first write to each 4 KB page takes a page fault, which costs about 2 µs.
2. The fbdev deferred-I/O work runs `fbdev_delay` jiffies later (1 jiffy, 4 ms at `CONFIG_HZ=250`),
   or at once when the app calls `fsync()`. It damages the rows from the lowest to the highest dirty
   page, at full width. A page holds 6.4 rows, so a change of 11 rows sends about 20.
3. The damage work copies the shadow into the GEM buffer and calls `panel_mipi_dbi_fb_dirty`.
4. `fb_dirty` copies the damaged rows out of the GEM buffer, swaps the bytes, and transposes them
   into `tbuf`, so the pixels go out along the panel's own scan lines.
5. It sends the address window (CASET and RASET), waits until the send can't cross the scan, then
   sends the pixels (RAMWR) with `spi_sync`.

The SPI clock is 50 MHz. The device tree asks for 60 MHz, but the VPU clock is fixed at 400 MHz and
`spi-bcm2835` only divides it by even numbers, so 400 / 8 is the fastest step below 60. The wire
takes 163 ns per byte, and a full frame (108,800 bytes) takes 17.56 ms.

## Two bugs the first version had

The TE wait never waited. The driver turned the TE interrupt off between frames with
`disable_irq()`. `pinctrl-bcm2835` has no `irq_disable` callback, so the kernel disables lazily. The
next TE edge marked the interrupt pending, and `enable_irq()` replayed that stale edge from a tasklet
36 µs later. Every send started at a random point in the refresh.

The copy was slow. `mipi_dbi_buf_copy` tells `drm_fb_swab` the GEM buffer is cached, so it loads
each pixel straight from write-combined memory. That took 6.7 ms for a full frame. With the 0.44 ms
transpose and the 17.6 ms send, a full frame took 25 ms, which capped full-screen updates at 40 fps.

## How the driver times a send

The panel scans its 320 lines in the order the driver sends them (MADCTL 0x90, with the ML bit
set). The TE pin is high while the panel blanks between refreshes. The driver keeps the TE interrupt
on while frames flow and timestamps both edges. From those it learns the refresh period T (19.76 ms
with FRCTRL2 0x15, so 50.6 Hz) and the blanking time b (0.58 ms).

The phase p of a send is the time since the last rising edge, when blanking starts. The scan reaches
line l at b + l × t, where the line time t is (T − b) / 320. A send to lines l1 to l2 that takes W
on the wire is safe in two cases:

- **Lead.** It starts before the scan reaches l1, so p ≤ b + l1 × t − m. The write must also cover
  a line faster than the scan does, W / (l2 − l1 + 1) < t, so the scan never catches it.
- **Trail.** It starts late enough that the scan passes l2 before the write gets there, so
  p ≥ b + (l2 + 1) × t − W + m. This refresh shows only old lines, and the next one shows only new
  ones.

The margin m is `te_margin_us`, 300 µs by default. Between the two cases, the driver sleeps with
`usleep_range` until the trail time and checks again. If the trail time falls after the end of the
refresh (the write is slower than the scan), it waits for the next blanking and leads instead. It
measures W from bytes and the recorded wire speed, which it updates after every send of 4 KB or more.

For a full frame at 50.6 Hz, lead ends at 0.28 ms and trail starts at 2.5 ms. For 60 rows
(6.3 ms on the wire), trail starts at 13.8 ms.

The CASET and RASET commands go out before the wait, so only RAMWR waits on the scan.

## The TE interrupt

The interrupt starts off (`IRQF_NO_AUTOEN`). The driver marks it `IRQ_DISABLE_UNLAZY`, so the chip
masks it at once and the kernel never replays an edge that came while it was off. The first send after a
pause turns it on and waits for a fresh blanking edge, which leads.

The handler runs in hard IRQ context. It filters the period with a 1/8 moving average and ignores
gaps more than 12.5% off. After `te_idle_edges` edges with no send (20 edges, about 200 ms), it turns
itself off with `disable_irq_nosync`. An idle panel then takes no TE interrupts. On the device, the
count in `/proc/interrupts` stays flat while the home screen sits idle.

## The present ioctl

`GH0ST_FBIO_PRESENT` is `_IO('F', 0xC0)`, 0x46C0. It runs the pending deferred-I/O work at once,
then waits for the damage work. It returns once everything written to the mapping is on the wire.
The driver adds it by copying the fbdev `fb_ops` and replacing `fb_ioctl` in its fbdev probe
wrapper. It doesn't call `drm_fb_helper_ioctl` for this ioctl, because that holds `fb_helper->lock`,
which the damage work takes. Every other ioctl goes to the helper as before.

M5's stock driver answers this ioctl with `ENOTTY`. gh0st tries it once when it opens the panel and
falls back to its 20 ms timer if it fails.

## How gh0st paces its animations

`crates/gh0st-ui/src/fb.rs` copies only the changed span of each row into the mapping, then calls
`fsync()`. While an animation runs on the home screen, `crates/gh0st-shell/src/run.rs` calls the
present ioctl after each frame that changed rows, then draws the next frame right away. Frames then
lock to the panel's refresh. When nothing moves, nothing is drawn and no timer runs.

`GH0ST_LOG=debug` logs, for each frame, the rows flushed and four times: wake to draw, draw, flush
and present. On the home screen, draw takes 0.5 ms and flush (copy plus `fsync`) 0.5 to 1.2 ms.

## Results

| | Before | After |
|---|---|---|
| First frame of a key press on the glass | 60.8 ms after the write | 9.5 ms |
| Frames of a 7-frame glide that reach the panel | 2 to 3 | all 7 |
| Glide send spacing | about 21 ms, timer-driven, out of step with the panel | 77 of 94 at 19.76 ms, one per refresh |
| Full frame: prep, RAMWR, total | 7.2, 17.6, 25.0 ms | 1.33, 17.56, 19.21 ms |
| Back-to-back full frames | 39.9 fps at a random phase | 50.8 fps, 153 frames in 3 s, every gap 19 to 20 ms |

The 17 glide sends that were not one refresh apart are each glide's first two. One leads in a
refresh and the next trails in the same refresh, so the panel still shows one new frame per
refresh.

In a blind A/B test (`scripts/ab-test.sh`, 5 rounds of scrolling stripes, sync off against on), the
viewer picked the synced version every round. Guessing gives 5 of 5 one time in 32.

## Module parameters

| Parameter | Default | Writable | What it does |
|---|---|---|---|
| `te` | Y | yes | Time sends against the scan. N sends at once, like the old driver. |
| `te_margin_us` | 300 | yes | Margin m around the scan when picking a send time. |
| `te_idle_edges` | 20 | yes | TE edges without a send before the interrupt turns off. |
| `te_vblank_high` | Y | yes | TE is high during blanking. Flip it if the panel inverts TE. |
| `te_timeout_ms` | 50 | yes | Longest wait for a TE edge before sending anyway. |
| `native_order` | Y | no | Send along the panel's scan lines (transposed). |
| `madctl_native` | 0x90 | no | MADCTL used with `native_order`. |
| `frctrl` | 0x15 | no | ST7789 FRCTRL2. 0x15 measures 50.6 Hz on this panel. |
| `fbdev_delay` | 1 | no | Jiffies fbdev waits after a write before sending. 0 keeps the kernel's 50 ms. |

The writable ones live in `/sys/module/panel_mipi_dbi_m/parameters/` and take effect on the next
send. The others need a module reload or a reboot.

## Stats

`/sys/bus/spi/devices/spi0.0/te_stats` shows:

- `te_on`, whether the TE interrupt is on.
- `period_ns` and `vblank_ns`, the filtered refresh period and blanking time.
- `wire_ns_per_kib`, the measured wire speed.
- `lead`, `trail` and `slept`, how many sends led, trailed, or slept first.
- `fresh`, how many times the interrupt turned back on.
- `blind`, how many sends found the phase unknown and waited for a blanking edge.
- `last_phase_ns`, the phase of the last send.

## Testing

- `scripts/tear-test.sh [seconds] [px per frame]` scrolls horizontal stripes at the panel rate.
  A tear shows as a vertical seam where the stripes step.
- `scripts/ab-test.sh [rounds] [seconds]` runs the same stripes twice per round, labelled A and B
  on the panel, with a driver parameter at two values in random order. It asks which tore less,
  then reveals the order. `AB_PARAM`, `AB_OLD` and `AB_NEW` pick the parameter and values.
- To see where time goes, trace one frame with ftrace:

  ```sh
  cd /sys/kernel/tracing
  echo panel_mipi_dbi_fb_dirty > set_graph_function
  printf '%s\n' panel_mipi_dbi_fb_dirty mipi_dbi_command_buf > set_ftrace_filter
  echo function_graph > current_tracer
  ```

  The last `mipi_dbi_command_buf` under each `fb_dirty` is the RAMWR.

## Install

The driver builds on the device against the running kernel:

```sh
cd os/kernel/panel-mipi-dbi-m && sudo ./install.sh   # then reboot
sudo ./install.sh uninstall                          # M5's driver back
```

## Open questions

- Whether any tearing remains. The A/B test shows a clear gain, but the viewer couldn't say whether
  the synced version is free of tears.
- The timing model assumes TE is high during blanking and that the scan follows the send order.
  Neither has been checked with a high-speed camera.
- The MADCTL tests (0x40 upside down, 0x80 tears, 0x90 tears least) ran while the TE wait was
  broken. They need repeating.
- The non-native path (`native_order=N`) only starts at blanking. It has no lead or trail model.
