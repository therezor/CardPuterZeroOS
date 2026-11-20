// SPDX-License-Identifier: GPL-2.0
/*
 * DRM driver for MIPI DBI compatible display panels
 *
 * Copyright 2022 Noralf Trønnes
 *
 * gh0st changes (CardputerZero, ST7789V 320x170 on SPI):
 * - TE sync: the TE IRQ timestamps both edges while frames flow, so the
 *   driver knows the refresh phase and period. A send starts where it cannot
 *   cross the scan: before the scan reaches its first line (lead), or late
 *   enough that the scan passes its last line first (trail). The IRQ turns
 *   itself off after te_idle_edges edges without a send.
 * - Native order: the panel is mounted rotated (MADCTL MV), so it refreshes
 *   across the rows we send. Transfers are transposed and sent along the
 *   panel's own scan lines, so a write never crosses the refresh.
 * - Refresh rate: FRCTRL2 slows the panel to 50 Hz, so a full 17 ms transfer
 *   at the 50 MHz SPI clock stays ahead of the 20 ms refresh.
 * - fbdev delay: the fbdev helper waits HZ / 20 (50 ms) after a write before
 *   it sends. One jiffy (4 ms at HZ=250) cuts that lag for every fbdev app.
 * - Present ioctl: GH0ST_FBIO_PRESENT returns once everything written to the
 *   fbdev mapping is on the wire, so a client can pace frames to the panel.
 * - Fast copy: rows are memcpy'd out of the write-combined GEM buffer, then
 *   swapped and transposed in one pass. Per-pixel loads from WC memory cost
 *   6.7 ms a frame.
 * Each is a module parameter, for tuning on the device.
 */

#include <linux/backlight.h>
#include <linux/delay.h>
#include <linux/firmware.h>
#include <linux/gpio/consumer.h>
#include <linux/irq.h>
#include <linux/ktime.h>
#include <linux/module.h>
#include <linux/of.h>
#include <linux/property.h>
#include <linux/regulator/consumer.h>
#include <linux/spi/spi.h>

#include <drm/clients/drm_client_setup.h>
#include <drm/drm_atomic_helper.h>
#include <drm/drm_damage_helper.h>
#include <drm/drm_drv.h>
#include <drm/drm_fb_helper.h>
#include <drm/drm_fourcc.h>
#include <drm/drm_framebuffer.h>
#include <drm/drm_gem_framebuffer_helper.h>
#include <drm/drm_fbdev_dma.h>
#include <drm/drm_gem_atomic_helper.h>
#include <drm/drm_gem_dma_helper.h>
#include <drm/drm_managed.h>
#include <drm/drm_mipi_dbi.h>
#include <drm/drm_modes.h>
#include <drm/drm_modeset_helper.h>

#include <video/mipi_display.h>


#ifndef MIPI_DCS_SET_TEAR_ON
#define MIPI_DCS_SET_TEAR_ON		0x35
#endif
#define MIPI_DCS_TEAR_MODE_VBLANK	0x00
#define ST7789_FRCTRL2			0xC6

static bool te = true;
module_param(te, bool, 0644);
MODULE_PARM_DESC(te, "Time each send against the panel scan, when te-gpios is set (default true)");

static bool native_order = true;
module_param(native_order, bool, 0444);
MODULE_PARM_DESC(native_order, "Send pixels along the panel's scan lines (default true)");

static uint madctl_native = 0x90;
module_param(madctl_native, uint, 0444);
MODULE_PARM_DESC(madctl_native, "MADCTL used with native_order (default 0x90)");

static uint frctrl = 0x15;
module_param(frctrl, uint, 0444);
MODULE_PARM_DESC(frctrl, "ST7789 FRCTRL2 value, 0x15 = 50.6 Hz measured, 0x0f = 60 Hz, 0 = keep (default 0x15)");

static uint te_timeout_ms = 50;
module_param(te_timeout_ms, uint, 0644);
MODULE_PARM_DESC(te_timeout_ms, "Longest wait for a TE edge before sending anyway (default 50)");

static uint te_margin_us = 300;
module_param(te_margin_us, uint, 0644);
MODULE_PARM_DESC(te_margin_us, "Safety margin around the scan when picking a send time (default 300)");

static uint te_idle_edges = 20;
module_param(te_idle_edges, uint, 0644);
MODULE_PARM_DESC(te_idle_edges, "TE edges without a send before the TE IRQ turns off (default 20)");

static bool te_vblank_high = true;
module_param(te_vblank_high, bool, 0644);
MODULE_PARM_DESC(te_vblank_high, "TE is high during vertical blanking (default true)");

/* Returns once the fbdev mapping's writes are on the wire. Stock fbdev: -ENOTTY. */
#define GH0ST_FBIO_PRESENT	_IO('F', 0xC0)

static uint fbdev_delay = 1;
module_param(fbdev_delay, uint, 0444);
MODULE_PARM_DESC(fbdev_delay, "Jiffies fbdev waits after a write before sending, 0 = upstream HZ/20 (default 1)");


struct panel_mipi_dbi_format {
    const char *name;
    u32 fourcc;
    unsigned int bpp;
};

static const struct panel_mipi_dbi_format panel_mipi_dbi_formats[] = {
    { "r5g6b5", DRM_FORMAT_RGB565, 16 },
    { "b6x2g6x2r6x2", DRM_FORMAT_RGB888, 24 },
};

/* dbidev must be the first member so container_of can cast it safely. */
struct panel_mipi_dbi_device {
    struct mipi_dbi_dev dbidev;

    struct gpio_desc *te;
    int te_irq;
    /* Completed on each vblank-start edge. */
    struct completion te_completion;
    /* TE state, written by the ISR. */
    spinlock_t te_lock;
    bool te_on;			/* IRQ enabled */
    unsigned int te_idle;		/* edges since the last send */
    ktime_t te_vbl;			/* last vblank-start edge, 0 = unknown */
    s64 te_period_ns;		/* filtered vblank-to-vblank time, 0 = unknown */
    s64 te_vblank_ns;		/* filtered blanking time */
    /* Measured wire time, ns per KiB. */
    u64 wire_ns_per_kib;
    /* Counters for te_stats. */
    u64 n_lead, n_trail, n_slept, n_fresh, n_blind;
    s64 last_phase_ns;
    /* Transposed copy of the damaged area, for native_order. */
    u16 *tbuf;
    /* One row of the framebuffer, for the fast copy. */
    u16 *line;
    struct mutex display_lock;		/* Serialize reset and display transfers */
};

static inline struct panel_mipi_dbi_device *
to_panel_mipi_dbi_device(struct mipi_dbi_dev *dbidev)
{
    return container_of(dbidev, struct panel_mipi_dbi_device, dbidev);
}

static int panel_mipi_dbi_get_format(struct device *dev, u32 *formats, unsigned int *bpp)
{
    const char *format_name;
    unsigned int i;
    int ret;

    formats[1] = DRM_FORMAT_XRGB8888;

    ret = device_property_read_string(dev, "format", &format_name);
    if (ret) {
        /* Old Device Trees don't have this property */
        formats[0] = DRM_FORMAT_RGB565;
        *bpp = 16;
        return 0;
    }

    for (i = 0; i < ARRAY_SIZE(panel_mipi_dbi_formats); i++) {
        const struct panel_mipi_dbi_format *format = &panel_mipi_dbi_formats[i];

        if (strcmp(format_name, format->name))
            continue;

        formats[0] = format->fourcc;
        *bpp = format->bpp;
        return 0;
    }

    dev_err(dev, "Pixel format is not supported: '%s'\n", format_name);

    return -EINVAL;
}

static const u8 panel_mipi_dbi_magic[15] = { 'M', 'I', 'P', 'I', ' ', 'D', 'B', 'I',
                         0, 0, 0, 0, 0, 0, 0 };

/*
 * The display controller configuration is stored in a firmware file.
 * The Device Tree 'compatible' property value with a '.bin' suffix is passed
 * to request_firmware() to fetch this file.
 */
struct panel_mipi_dbi_config {
    /* Magic string: panel_mipi_dbi_magic */
    u8 magic[15];

    /* Config file format version */
    u8 file_format_version;

    /*
     * MIPI commands to execute when the display pipeline is enabled.
     * This is used to configure the display controller.
     *
     * The commands are stored in a byte array with the format:
     *     command, num_parameters, [ parameter, ...], command, ...
     *
     * Some commands require a pause before the next command can be received.
     * Inserting a delay in the command sequence is done by using the NOP command with one
     * parameter: delay in miliseconds (the No Operation command is part of the MIPI Display
     * Command Set where it has no parameters).
     *
     * Example:
     *     command 0x11
     *     sleep 120ms
     *     command 0xb1 parameters 0x01, 0x2c, 0x2d
     *     command 0x29
     *
     * Byte sequence:
     *     0x11 0x00
     *     0x00 0x01 0x78
     *     0xb1 0x03 0x01 0x2c 0x2d
     *     0x29 0x00
     */
    u8 commands[];
};

struct panel_mipi_dbi_commands {
    const u8 *buf;
    size_t len;
};

static struct panel_mipi_dbi_commands *
panel_mipi_dbi_check_commands(struct device *dev, const struct firmware *fw)
{
    const struct panel_mipi_dbi_config *config = (struct panel_mipi_dbi_config *)fw->data;
    struct panel_mipi_dbi_commands *commands;
    size_t size = fw->size, commands_len;
    unsigned int i = 0;

    if (size < sizeof(*config) + 2) { /* At least 1 command */
        dev_err(dev, "config: file size=%zu is too small\n", size);
        return ERR_PTR(-EINVAL);
    }

    if (memcmp(config->magic, panel_mipi_dbi_magic, sizeof(config->magic))) {
        dev_err(dev, "config: Bad magic: %15ph\n", config->magic);
        return ERR_PTR(-EINVAL);
    }

    if (config->file_format_version != 1) {
        dev_err(dev, "config: version=%u is not supported\n", config->file_format_version);
        return ERR_PTR(-EINVAL);
    }

    drm_dev_dbg(dev, DRM_UT_DRIVER, "size=%zu version=%u\n", size, config->file_format_version);

    commands_len = size - sizeof(*config);

    while ((i + 1) < commands_len) {
        u8 command = config->commands[i++];
        u8 num_parameters = config->commands[i++];
        const u8 *parameters = &config->commands[i];

        i += num_parameters;
        if (i > commands_len) {
            dev_err(dev, "config: command=0x%02x num_parameters=%u overflows\n",
                command, num_parameters);
            return ERR_PTR(-EINVAL);
        }

        if (command == 0x00 && num_parameters == 1)
            drm_dev_dbg(dev, DRM_UT_DRIVER, "sleep %ums\n", parameters[0]);
        else
            drm_dev_dbg(dev, DRM_UT_DRIVER, "command %02x %*ph\n",
                    command, num_parameters, parameters);
    }

    if (i != commands_len) {
        dev_err(dev, "config: malformed command array\n");
        return ERR_PTR(-EINVAL);
    }

    commands = devm_kzalloc(dev, sizeof(*commands), GFP_KERNEL);
    if (!commands)
        return ERR_PTR(-ENOMEM);

    commands->len = commands_len;
    commands->buf = devm_kmemdup(dev, config->commands, commands->len, GFP_KERNEL);
    if (!commands->buf)
        return ERR_PTR(-ENOMEM);

    return commands;
}

static struct panel_mipi_dbi_commands *panel_mipi_dbi_commands_from_fw(struct device *dev)
{
    struct panel_mipi_dbi_commands *commands;
    const struct firmware *fw;
    const char *compatible;
    char fw_name[40];
    int ret;

    ret = of_property_read_string_index(dev->of_node, "compatible", 0, &compatible);
    if (ret)
        return ERR_PTR(ret);

    snprintf(fw_name, sizeof(fw_name), "%s.bin", compatible);
    ret = request_firmware(&fw, fw_name, dev);
    if (ret) {
        dev_err(dev, "No config file found for compatible '%s' (error=%d)\n",
            compatible, ret);

        return ERR_PTR(ret);
    }

    commands = panel_mipi_dbi_check_commands(dev, fw);
    release_firmware(fw);

    return commands;
}

static void panel_mipi_dbi_commands_execute(struct mipi_dbi *dbi,
                        struct panel_mipi_dbi_commands *commands)
{
    unsigned int i = 0;

    if (!commands)
        return;

    while (i < commands->len) {
        u8 command = commands->buf[i++];
        u8 num_parameters = commands->buf[i++];
        const u8 *parameters = &commands->buf[i];

        if (command == 0x00 && num_parameters == 1)
            msleep(parameters[0]);
        else if (num_parameters)
            mipi_dbi_command_stackbuf(dbi, command, parameters, num_parameters);
        else
            mipi_dbi_command(dbi, command);

        i += num_parameters;
    }
}

static bool panel_mipi_dbi_native(struct panel_mipi_dbi_device *panel)
{
    return native_order && panel->tbuf && panel->dbidev.pixel_format == DRM_FORMAT_RGB565;
}

/* Settings applied after the firmware init sequence, every time it runs. */
static void panel_mipi_dbi_tune(struct panel_mipi_dbi_device *panel)
{
    struct mipi_dbi *dbi = &panel->dbidev.dbi;

    if (panel_mipi_dbi_native(panel))
        mipi_dbi_command(dbi, MIPI_DCS_SET_ADDRESS_MODE, madctl_native & 0xff);
    if (frctrl)
        mipi_dbi_command(dbi, ST7789_FRCTRL2, frctrl & 0xff);
    if (panel->te)
        mipi_dbi_command(dbi, MIPI_DCS_SET_TEAR_ON, MIPI_DCS_TEAR_MODE_VBLANK);
}

static int panel_mipi_dbi_reinitialize(struct panel_mipi_dbi_device *panel)
{
    struct mipi_dbi_dev *dbidev = &panel->dbidev;
    struct mipi_dbi *dbi = &dbidev->dbi;
    int ret;

    /* Balance the temporary regulator references after initialization. */
    ret = mipi_dbi_poweron_reset(dbidev);
    if (ret)
        return ret;

    panel_mipi_dbi_commands_execute(dbi, dbidev->driver_private);
    panel_mipi_dbi_tune(panel);

    if (dbidev->regulator)
        regulator_disable(dbidev->regulator);
    if (dbidev->io_regulator)
        regulator_disable(dbidev->io_regulator);

    return 0;
}

static ssize_t panel_mipi_dbi_reset_store(struct device *dev,
                      struct device_attribute *attr,
                      const char *buf, size_t count)
{
    struct spi_device *spi = to_spi_device(dev);
    struct drm_device *drm = spi_get_drvdata(spi);
    struct panel_mipi_dbi_device *panel;
    unsigned int value;
    int idx, ret;

    ret = kstrtouint(buf, 0, &value);
    if (ret)
        return ret;

    if (value != 1)
        return -EINVAL;

    if (!drm)
        return -ENODEV;

    panel = to_panel_mipi_dbi_device(drm_to_mipi_dbi_dev(drm));

    if (!drm_dev_enter(drm, &idx))
        return -ENODEV;

    mutex_lock(&panel->display_lock);
    ret = panel_mipi_dbi_reinitialize(panel);
    mutex_unlock(&panel->display_lock);

    drm_dev_exit(idx);

    return ret ? ret : count;
}

static DEVICE_ATTR(reset, 0200, NULL, panel_mipi_dbi_reset_store);

static ssize_t te_stats_show(struct device *dev, struct device_attribute *attr, char *buf)
{
    struct drm_device *drm = spi_get_drvdata(to_spi_device(dev));
    struct panel_mipi_dbi_device *panel;

    if (!drm)
        return -ENODEV;
    panel = to_panel_mipi_dbi_device(drm_to_mipi_dbi_dev(drm));
    return sysfs_emit(buf,
              "te_on %d\nperiod_ns %lld\nvblank_ns %lld\nwire_ns_per_kib %llu\n"
              "lead %llu\ntrail %llu\nslept %llu\nfresh %llu\nblind %llu\n"
              "last_phase_ns %lld\n",
              panel->te_on, panel->te_period_ns, panel->te_vblank_ns,
              panel->wire_ns_per_kib, panel->n_lead, panel->n_trail,
              panel->n_slept, panel->n_fresh, panel->n_blind, panel->last_phase_ns);
}

static DEVICE_ATTR_RO(te_stats);

static struct attribute *panel_mipi_dbi_attrs[] = {
    &dev_attr_reset.attr,
    &dev_attr_te_stats.attr,
    NULL,
};

static const struct attribute_group panel_mipi_dbi_attr_group = {
    .attrs = panel_mipi_dbi_attrs,
};

/*
 * TE edge, in hard IRQ context. Tracks the refresh period and the blanking
 * width, wakes a waiter on vblank start, and turns the IRQ off when no send
 * has happened for te_idle_edges edges, so an idle panel costs no wakeups.
 */
static irqreturn_t panel_mipi_dbi_te_isr(int irq, void *data)
{
    struct panel_mipi_dbi_device *panel = data;
    ktime_t now = ktime_get();
    bool high = gpiod_get_value(panel->te);
    bool vbl_start = high == te_vblank_high;
    s64 d;

    spin_lock(&panel->te_lock);
    if (vbl_start) {
        if (panel->te_vbl) {
            d = ktime_to_ns(ktime_sub(now, panel->te_vbl));
            if (!panel->te_period_ns) {
                if (d > 8 * NSEC_PER_MSEC && d < 50 * NSEC_PER_MSEC)
                    panel->te_period_ns = d;
            } else if (abs(d - panel->te_period_ns) < panel->te_period_ns / 8) {
                panel->te_period_ns += (d - panel->te_period_ns) / 8;
            }
        }
        panel->te_vbl = now;
        complete(&panel->te_completion);
    } else if (panel->te_vbl && panel->te_period_ns) {
        d = ktime_to_ns(ktime_sub(now, panel->te_vbl));
        if (d > 0 && d < panel->te_period_ns / 2)
            panel->te_vblank_ns = panel->te_vblank_ns ?
                panel->te_vblank_ns + (d - panel->te_vblank_ns) / 8 : d;
    }
    if (panel->te_on && ++panel->te_idle > te_idle_edges) {
        panel->te_on = false;
        panel->te_vbl = 0;
        disable_irq_nosync(irq);
    }
    spin_unlock(&panel->te_lock);

    return IRQ_HANDLED;
}

static void panel_mipi_dbi_wait_vbl(struct panel_mipi_dbi_device *panel)
{
    if (!wait_for_completion_timeout(&panel->te_completion,
                     msecs_to_jiffies(te_timeout_ms)))
        dev_warn_once(panel->dbidev.drm.dev, "Timeout waiting for TE signal\n");
}

/*
 * Waits until a send of `bytes` to scan lines l1..l2 cannot cross the scan.
 * Phase p runs from vblank start. The scan reaches line l at b + l * t_l,
 * with b the blanking time and t_l the line time. A send is safe if it starts
 * before the scan reaches l1 (and the write per line is faster than the
 * scan), or if the scan passes l2 before the write gets there.
 */
static void panel_mipi_dbi_wait_for_scan(struct panel_mipi_dbi_device *panel,
                     unsigned int l1, unsigned int l2, size_t bytes,
                     unsigned int lines)
{
    s64 margin = (s64)te_margin_us * NSEC_PER_USEC;
    unsigned long flags;
    bool fresh = false;
    int tries;

    if (!panel->te || !te)
        return;

    spin_lock_irqsave(&panel->te_lock, flags);
    panel->te_idle = 0;
    if (!panel->te_on) {
        panel->te_on = true;
        panel->te_vbl = 0;
        reinit_completion(&panel->te_completion);
        fresh = true;
    }
    spin_unlock_irqrestore(&panel->te_lock, flags);

    if (fresh) {
        enable_irq(panel->te_irq);
        panel_mipi_dbi_wait_vbl(panel);
        panel->n_fresh++;
    }

    for (tries = 0; tries < 4; tries++) {
        s64 period, b, tl, tw, p, lead_end, trail_start;
        ktime_t vbl;

        spin_lock_irqsave(&panel->te_lock, flags);
        vbl = panel->te_vbl;
        period = panel->te_period_ns;
        b = panel->te_vblank_ns;
        spin_unlock_irqrestore(&panel->te_lock, flags);

        p = vbl ? ktime_to_ns(ktime_sub(ktime_get(), vbl)) : -1;
        if (!vbl || !period || !b || p > 2 * period) {
            /* Phase unknown: start on a fresh vblank, which leads the scan. */
            reinit_completion(&panel->te_completion);
            panel_mipi_dbi_wait_vbl(panel);
            panel->n_blind++;
            return;
        }
        p %= period;
        panel->last_phase_ns = p;

        tl = div_s64(period - b, lines);
        tw = div_u64((u64)bytes * panel->wire_ns_per_kib, 1024);
        lead_end = b + l1 * tl - margin;
        trail_start = b + (l2 + 1) * tl - tw + margin;

        /* Leading only works if the write per line beats the scan. */
        if (p <= lead_end && div_s64(tw, l2 - l1 + 1) < tl) {
            panel->n_lead++;
            return;
        }
        if (p >= trail_start && trail_start < period) {
            panel->n_trail++;
            return;
        }
        panel->n_slept++;
        if (trail_start < period) {
            s64 us = div_s64(trail_start - p, NSEC_PER_USEC);

            usleep_range(us, us + 50);
        } else {
            /* The write is too slow to trail; lead from the next vblank. */
            reinit_completion(&panel->te_completion);
            panel_mipi_dbi_wait_vbl(panel);
        }
    }
}

/* Called after each send, so the IRQ stays on while frames flow. */
static void panel_mipi_dbi_sent(struct panel_mipi_dbi_device *panel, size_t bytes, s64 ns)
{
    unsigned long flags;

    if (bytes >= 4096 && ns > 0) {
        u64 per_kib = div_u64((u64)ns * 1024, bytes);

        panel->wire_ns_per_kib = panel->wire_ns_per_kib ?
            panel->wire_ns_per_kib - panel->wire_ns_per_kib / 8 + per_kib / 8 : per_kib;
    }
    if (!panel->te)
        return;
    spin_lock_irqsave(&panel->te_lock, flags);
    panel->te_idle = 0;
    spin_unlock_irqrestore(&panel->te_lock, flags);
}

/*
 * Swaps and transposes an RGB565 rect straight into tbuf. Each row is copied
 * out of the GEM buffer with memcpy first: it is write-combined, and per-pixel
 * loads from it are slow. False if the fast path does not apply.
 */
static bool panel_mipi_dbi_fast_copy(struct panel_mipi_dbi_device *panel,
                     struct iosys_map *src, struct drm_framebuffer *fb,
                     struct drm_rect *rect)
{
    unsigned int w = rect->x2 - rect->x1;
    unsigned int h = rect->y2 - rect->y1;
    bool swap = panel->dbidev.dbi.swap_bytes;
    unsigned int pitch = fb->pitches[0];
    const u8 *row;
    unsigned int x, y;

    if (!panel->line || fb->format->format != DRM_FORMAT_RGB565 || src->is_iomem ||
        drm_gem_is_imported(drm_gem_fb_get_obj(fb, 0)))
        return false;

    row = (const u8 *)src->vaddr + rect->y1 * pitch + rect->x1 * 2;
    for (y = 0; y < h; y++, row += pitch) {
        u16 *dst = panel->tbuf + y;

        memcpy(panel->line, row, w * 2);
        if (swap)
            for (x = 0; x < w; x++)
                dst[x * h] = swab16(panel->line[x]);
        else
            for (x = 0; x < w; x++)
                dst[x * h] = panel->line[x];
    }
    return true;
}

/*
 * Sends one damaged rectangle. Native order transposes it so pixels go out
 * along the panel's scan lines: column y + top_offset of scan line x. With
 * MADCTL 0x80 that lands where MADCTL 0xA0 put landscape pixel (x, y), but
 * the address counter now walks the scan line. ML (0x10) makes the panel
 * refresh in the order the lines are sent, so the write leads the refresh.
 * Measured on the device: 0x40 is upside down, 0x80 still tears, 0x90 is
 * the best.
 */
static void panel_mipi_dbi_fb_dirty(struct iosys_map *src, struct drm_framebuffer *fb,
                    struct drm_rect *rect,
                    struct drm_format_conv_state *fmtcnv_state)
{
    struct mipi_dbi_dev *dbidev = drm_to_mipi_dbi_dev(fb->dev);
    struct panel_mipi_dbi_device *panel = to_panel_mipi_dbi_device(dbidev);
    struct mipi_dbi *dbi = &dbidev->dbi;
    unsigned int w = rect->x2 - rect->x1;
    unsigned int h = rect->y2 - rect->y1;
    unsigned int xs, xe, ys, ye, x, y;
    u16 *tx = dbidev->tx_buf;
    void *out = tx;
    size_t bytes;
    ktime_t t0;
    int ret = 0;

    if (panel_mipi_dbi_native(panel)) {
        if (!panel_mipi_dbi_fast_copy(panel, src, fb, rect)) {
            ret = mipi_dbi_buf_copy(tx, src, fb, rect, dbi->swap_bytes, fmtcnv_state);
            if (ret)
                goto err;
            for (x = 0; x < w; x++)
                for (y = 0; y < h; y++)
                    panel->tbuf[x * h + y] = tx[y * w + x];
        }
        out = panel->tbuf;
        xs = rect->y1 + dbidev->top_offset;
        xe = rect->y2 - 1 + dbidev->top_offset;
        ys = rect->x1 + dbidev->left_offset;
        ye = rect->x2 - 1 + dbidev->left_offset;
    } else {
        ret = mipi_dbi_buf_copy(tx, src, fb, rect, dbi->swap_bytes, fmtcnv_state);
        if (ret)
            goto err;
        xs = rect->x1 + dbidev->left_offset;
        xe = rect->x2 - 1 + dbidev->left_offset;
        ys = rect->y1 + dbidev->top_offset;
        ye = rect->y2 - 1 + dbidev->top_offset;
    }
    bytes = drm_format_info_min_pitch(drm_format_info(dbidev->pixel_format), 0, w) * h;

    /* The address window goes first, so only RAMWR waits on the scan. */
    mipi_dbi_command(dbi, MIPI_DCS_SET_COLUMN_ADDRESS, (xs >> 8) & 0xff, xs & 0xff,
             (xe >> 8) & 0xff, xe & 0xff);
    mipi_dbi_command(dbi, MIPI_DCS_SET_PAGE_ADDRESS, (ys >> 8) & 0xff, ys & 0xff,
             (ye >> 8) & 0xff, ye & 0xff);

    if (panel_mipi_dbi_native(panel))
        panel_mipi_dbi_wait_for_scan(panel, ys, ye, bytes, fb->width);
    else
        panel_mipi_dbi_wait_for_scan(panel, 0, 0, 0, 1);

    t0 = ktime_get();
    ret = mipi_dbi_command_buf(dbi, MIPI_DCS_WRITE_MEMORY_START, out, bytes);
    panel_mipi_dbi_sent(panel, bytes, ktime_to_ns(ktime_sub(ktime_get(), t0)));
err:
    if (ret)
        drm_err_once(fb->dev, "Failed to update display %d\n", ret);
}

static void panel_mipi_dbi_enable_flush(struct mipi_dbi_dev *dbidev,
                    struct drm_plane_state *plane_state)
{
    struct drm_shadow_plane_state *shadow = to_drm_shadow_plane_state(plane_state);
    struct drm_framebuffer *fb = plane_state->fb;
    struct drm_rect rect = { .x1 = 0, .x2 = fb->width, .y1 = 0, .y2 = fb->height };
    int idx;

    if (!drm_dev_enter(&dbidev->drm, &idx))
        return;

    panel_mipi_dbi_fb_dirty(&shadow->data[0], fb, &rect, &shadow->fmtcnv_state);
    backlight_enable(dbidev->backlight);

    drm_dev_exit(idx);
}

static void panel_mipi_dbi_enable(struct drm_simple_display_pipe *pipe,
                  struct drm_crtc_state *crtc_state,
                  struct drm_plane_state *plane_state)
{
    struct mipi_dbi_dev *dbidev = drm_to_mipi_dbi_dev(pipe->crtc.dev);
    struct panel_mipi_dbi_device *panel = to_panel_mipi_dbi_device(dbidev);
    struct mipi_dbi *dbi = &dbidev->dbi;
    int ret, idx;

    if (!drm_dev_enter(pipe->crtc.dev, &idx))
        return;

    drm_dbg(pipe->crtc.dev, "\n");

    mutex_lock(&panel->display_lock);

    ret = mipi_dbi_poweron_conditional_reset(dbidev);
    if (ret < 0)
        goto out_exit;
    if (!ret)
        panel_mipi_dbi_commands_execute(dbi, dbidev->driver_private);
    panel_mipi_dbi_tune(panel);

    panel_mipi_dbi_enable_flush(dbidev, plane_state);
out_exit:
    mutex_unlock(&panel->display_lock);
    drm_dev_exit(idx);
}

/* Update callback remains locked to serialize normal transfers with reset. */
static void panel_mipi_dbi_pipe_update(struct drm_simple_display_pipe *pipe,
                       struct drm_plane_state *old_state)
{
    struct mipi_dbi_dev *dbidev = drm_to_mipi_dbi_dev(pipe->crtc.dev);
    struct panel_mipi_dbi_device *panel = to_panel_mipi_dbi_device(dbidev);

    struct drm_plane_state *state = pipe->plane.state;
    struct drm_shadow_plane_state *shadow = to_drm_shadow_plane_state(state);
    struct drm_framebuffer *fb = state->fb;
    struct drm_rect rect;
    int idx;

    if (!pipe->crtc.state->active || WARN_ON(!fb))
        return;
    if (!drm_dev_enter(fb->dev, &idx))
        return;

    mutex_lock(&panel->display_lock);
    if (drm_atomic_helper_damage_merged(old_state, state, &rect))
        panel_mipi_dbi_fb_dirty(&shadow->data[0], fb, &rect, &shadow->fmtcnv_state);
    mutex_unlock(&panel->display_lock);

    drm_dev_exit(idx);
}

static void panel_mipi_dbi_disable(struct drm_simple_display_pipe *pipe)
{
    struct mipi_dbi_dev *dbidev = drm_to_mipi_dbi_dev(pipe->crtc.dev);
    struct panel_mipi_dbi_device *panel = to_panel_mipi_dbi_device(dbidev);

    mutex_lock(&panel->display_lock);
    mipi_dbi_pipe_disable(pipe);
    mutex_unlock(&panel->display_lock);
}

static const struct drm_simple_display_pipe_funcs panel_mipi_dbi_pipe_funcs = {
    DRM_MIPI_DBI_SIMPLE_DISPLAY_PIPE_FUNCS(panel_mipi_dbi_enable),
    .disable = panel_mipi_dbi_disable,
    .update = panel_mipi_dbi_pipe_update,
};

DEFINE_DRM_GEM_DMA_FOPS(panel_mipi_dbi_fops);

#ifdef CONFIG_DRM_FBDEV_EMULATION
static struct fb_ops panel_mipi_dbi_fb_ops;
static int (*panel_mipi_dbi_helper_ioctl)(struct fb_info *info, unsigned int cmd,
                      unsigned long arg);

/*
 * GH0ST_FBIO_PRESENT runs the pending deferred I/O now and waits for the
 * damage work, so it returns once the frame is on the wire. It must not call
 * drm_fb_helper_ioctl: that holds fb_helper->lock, which the damage blit takes.
 */
static int panel_mipi_dbi_fb_ioctl(struct fb_info *info, unsigned int cmd, unsigned long arg)
{
    struct drm_fb_helper *helper = info->par;

    if (cmd == GH0ST_FBIO_PRESENT) {
        if (info->fbdefio)
            flush_delayed_work(&info->deferred_work);
        flush_work(&helper->damage_work);
        return 0;
    }
    return panel_mipi_dbi_helper_ioctl ? panel_mipi_dbi_helper_ioctl(info, cmd, arg) : -ENOTTY;
}

/*
 * Wraps the DMA fbdev probe. It shortens the deferred-I/O delay (the helper
 * reads fbdefio.delay each time it schedules a send) and adds the present
 * ioctl. fbops is set before the fb_info is registered, so swapping it is safe.
 */
static int panel_mipi_dbi_fbdev_probe(struct drm_fb_helper *fb_helper,
                      struct drm_fb_helper_surface_size *sizes)
{
    struct fb_info *info;
    int ret = drm_fbdev_dma_driver_fbdev_probe(fb_helper, sizes);

    if (ret)
        return ret;
    info = fb_helper->info;
    if (fbdev_delay && info->fbdefio)
        fb_helper->fbdefio.delay = fbdev_delay;
    if (info->fbops != &panel_mipi_dbi_fb_ops) {
        panel_mipi_dbi_fb_ops = *info->fbops;
        panel_mipi_dbi_helper_ioctl = info->fbops->fb_ioctl;
        panel_mipi_dbi_fb_ops.fb_ioctl = panel_mipi_dbi_fb_ioctl;
        panel_mipi_dbi_fb_ops.owner = THIS_MODULE;
        info->fbops = &panel_mipi_dbi_fb_ops;
    }
    return 0;
}
#define PANEL_MIPI_DBI_FBDEV_OPS .fbdev_probe = panel_mipi_dbi_fbdev_probe
#else
#define PANEL_MIPI_DBI_FBDEV_OPS DRM_FBDEV_DMA_DRIVER_OPS
#endif

static const struct drm_driver panel_mipi_dbi_driver = {
    .driver_features	= DRIVER_GEM | DRIVER_MODESET | DRIVER_ATOMIC,
    .fops			= &panel_mipi_dbi_fops,
    DRM_GEM_DMA_DRIVER_OPS_VMAP,
    PANEL_MIPI_DBI_FBDEV_OPS,
    .debugfs_init		= mipi_dbi_debugfs_init,
    .name			= "panel-mipi-dbi",
    .desc			= "MIPI DBI compatible display panel",
    .major			= 1,
    .minor			= 0,
};

static int panel_mipi_dbi_get_mode(struct mipi_dbi_dev *dbidev, struct drm_display_mode *mode)
{
    struct device *dev = dbidev->drm.dev;
    u16 hback_porch, vback_porch;
    int ret;

    ret = of_get_drm_panel_display_mode(dev->of_node, mode, NULL);
    if (ret) {
        dev_err(dev, "%pOF: failed to get panel-timing (error=%d)\n", dev->of_node, ret);
        return ret;
    }

    mode->type = DRM_MODE_TYPE_DRIVER | DRM_MODE_TYPE_PREFERRED;

    hback_porch = mode->htotal - mode->hsync_end;
    vback_porch = mode->vtotal - mode->vsync_end;

    /*
     * Make sure width and height are set and that only back porch and
     * pixelclock are set in the other timing values. Also check that
     * width and height don't exceed the 16-bit value specified by MIPI DCS.
     */
    if (!mode->hdisplay || !mode->vdisplay || mode->flags ||
        mode->hsync_end > mode->hdisplay || (hback_porch + mode->hdisplay) > 0xffff ||
        mode->vsync_end > mode->vdisplay || (vback_porch + mode->vdisplay) > 0xffff) {
        dev_err(dev, "%pOF: panel-timing out of bounds\n", dev->of_node);
        return -EINVAL;
    }

    /* The driver doesn't use the pixel clock but it is mandatory so fake one if not set */
    if (!mode->clock)
        mode->clock = mode->htotal * mode->vtotal * 60 / 1000;

    dbidev->top_offset = vback_porch;
    dbidev->left_offset = hback_porch;

    return 0;
}

static int panel_mipi_dbi_spi_probe(struct spi_device *spi)
{
    struct device *dev = &spi->dev;
    struct panel_mipi_dbi_device *panel;
    struct drm_display_mode mode;
    struct mipi_dbi_dev *dbidev;
    struct drm_device *drm;
    struct mipi_dbi *dbi;
    struct gpio_desc *dc;
    unsigned int bpp;
    size_t buf_size;
    u32 formats[2];
    int ret;

    panel = devm_drm_dev_alloc(dev, &panel_mipi_dbi_driver,
                   struct panel_mipi_dbi_device, dbidev.drm);
    if (IS_ERR(panel))
        return PTR_ERR(panel);

    dbidev = &panel->dbidev;
    dbi = &dbidev->dbi;
    drm = &dbidev->drm;
    mutex_init(&panel->display_lock);

    ret = panel_mipi_dbi_get_mode(dbidev, &mode);
    if (ret)
        return ret;

    dbidev->regulator = devm_regulator_get(dev, "power");
    if (IS_ERR(dbidev->regulator))
        return dev_err_probe(dev, PTR_ERR(dbidev->regulator),
                     "Failed to get regulator 'power'\n");

    dbidev->io_regulator = devm_regulator_get(dev, "io");
    if (IS_ERR(dbidev->io_regulator))
        return dev_err_probe(dev, PTR_ERR(dbidev->io_regulator),
                     "Failed to get regulator 'io'\n");

    dbidev->backlight = devm_of_find_backlight(dev);
    if (IS_ERR(dbidev->backlight))
        return dev_err_probe(dev, PTR_ERR(dbidev->backlight), "Failed to get backlight\n");

    dbi->reset = devm_gpiod_get_optional(dev, "reset", GPIOD_OUT_HIGH);
    if (IS_ERR(dbi->reset))
        return dev_err_probe(dev, PTR_ERR(dbi->reset), "Failed to get GPIO 'reset'\n");

    /* Multiple panels can share the "dc" GPIO, but only if they are on the same SPI bus! */
    dc = devm_gpiod_get_optional(dev, "dc", GPIOD_OUT_LOW | GPIOD_FLAGS_BIT_NONEXCLUSIVE);
    if (IS_ERR(dc))
        return dev_err_probe(dev, PTR_ERR(dc), "Failed to get GPIO 'dc'\n");

    ret = mipi_dbi_spi_init(spi, dbi, dc);
    if (ret)
        return ret;

    if (device_property_present(dev, "write-only"))
        dbi->read_commands = NULL;

    /* Get the optional TE pin and request its IRQ */
    panel->te = devm_gpiod_get_optional(dev, "te", GPIOD_IN);
    if (IS_ERR(panel->te))
        return dev_err_probe(dev, PTR_ERR(panel->te), "Failed to get GPIO 'te'\n");

    if (panel->te) {
        init_completion(&panel->te_completion);
        spin_lock_init(&panel->te_lock);

        panel->te_irq = gpiod_to_irq(panel->te);
        if (panel->te_irq < 0)
            return dev_err_probe(dev, panel->te_irq,
                         "Failed to get TE irq\n");

        /*
         * Off until a send needs it (IRQF_NO_AUTOEN). Unlazy, so an edge that
         * comes while it is off is dropped, not replayed on enable_irq.
         */
        irq_set_status_flags(panel->te_irq, IRQ_DISABLE_UNLAZY);
        ret = devm_request_irq(dev, panel->te_irq, panel_mipi_dbi_te_isr,
                       IRQF_TRIGGER_RISING | IRQF_TRIGGER_FALLING | IRQF_NO_AUTOEN,
                       dev_name(dev), panel);
        if (ret)
            return dev_err_probe(dev, ret, "Failed to request TE irq\n");
    }

    dbidev->driver_private = panel_mipi_dbi_commands_from_fw(dev);
    if (IS_ERR(dbidev->driver_private))
        return PTR_ERR(dbidev->driver_private);

    ret = panel_mipi_dbi_get_format(dev, formats, &bpp);
    if (ret)
        return ret;

    buf_size = DIV_ROUND_UP(mode.hdisplay * mode.vdisplay * bpp, 8);
    panel->tbuf = devm_kmalloc(dev, buf_size, GFP_KERNEL);
    if (!panel->tbuf)
        return -ENOMEM;
    panel->line = devm_kmalloc(dev, mode.hdisplay * sizeof(u16), GFP_KERNEL);
    if (!panel->line)
        return -ENOMEM;
    /* 50 MHz SPI until a send measures it. */
    panel->wire_ns_per_kib = 160 * 1024;
    ret = mipi_dbi_dev_init_with_formats(dbidev, &panel_mipi_dbi_pipe_funcs,
                         formats, ARRAY_SIZE(formats),
                         &mode, 0, buf_size);
    if (ret)
        return ret;

    drm_mode_config_reset(drm);

    ret = drm_dev_register(drm, 0);
    if (ret)
        return ret;

    spi_set_drvdata(spi, drm);

    ret = devm_device_add_group(dev, &panel_mipi_dbi_attr_group);
    if (ret) {
        drm_dev_unplug(drm);
        drm_atomic_helper_shutdown(drm);
        return ret;
    }

    if (bpp == 16)
        drm_client_setup_with_fourcc(drm, DRM_FORMAT_RGB565);
    else
        drm_client_setup_with_fourcc(drm, DRM_FORMAT_RGB888);

    return 0;
}

static void panel_mipi_dbi_spi_remove(struct spi_device *spi)
{
    struct drm_device *drm = spi_get_drvdata(spi);

    drm_dev_unplug(drm);
    drm_atomic_helper_shutdown(drm);
}

static void panel_mipi_dbi_spi_shutdown(struct spi_device *spi)
{
    drm_atomic_helper_shutdown(spi_get_drvdata(spi));
}

static int __maybe_unused panel_mipi_dbi_pm_suspend(struct device *dev)
{
    return drm_mode_config_helper_suspend(dev_get_drvdata(dev));
}

static int __maybe_unused panel_mipi_dbi_pm_resume(struct device *dev)
{
    drm_mode_config_helper_resume(dev_get_drvdata(dev));

    return 0;
}

static const struct dev_pm_ops panel_mipi_dbi_pm_ops = {
    SET_SYSTEM_SLEEP_PM_OPS(panel_mipi_dbi_pm_suspend, panel_mipi_dbi_pm_resume)
};

static const struct of_device_id panel_mipi_dbi_spi_of_match[] = {
    { .compatible = "panel-mipi-dbi-spi" },
    { .compatible = "panel-mipi-dbi-m5stack" },
    {},
};
MODULE_DEVICE_TABLE(of, panel_mipi_dbi_spi_of_match);

static const struct spi_device_id panel_mipi_dbi_spi_id[] = {
    { "panel-mipi-dbi-spi", 0 },
    { "panel-mipi-dbi-m5stack", 0 },
    { },
};
MODULE_DEVICE_TABLE(spi, panel_mipi_dbi_spi_id);

static struct spi_driver panel_mipi_dbi_spi_driver = {
    .driver = {
        .name = "panel-mipi-dbi-spi",
        .of_match_table = panel_mipi_dbi_spi_of_match,
        .pm = &panel_mipi_dbi_pm_ops,
    },
    .id_table = panel_mipi_dbi_spi_id,
    .probe = panel_mipi_dbi_spi_probe,
    .remove = panel_mipi_dbi_spi_remove,
    .shutdown = panel_mipi_dbi_spi_shutdown,
};
module_spi_driver(panel_mipi_dbi_spi_driver);

MODULE_DESCRIPTION("MIPI DBI compatible display panel driver");
MODULE_AUTHOR("Noralf Trønnes");
MODULE_LICENSE("GPL");
