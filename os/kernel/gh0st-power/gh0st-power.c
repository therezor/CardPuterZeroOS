// SPDX-License-Identifier: GPL-2.0
/*
 * The CardputerZero power switch. Replaces M5's powerfail_suo, which
 * remounted every filesystem read-only 500 ms after the flip and left the
 * rest to a 2 s timer.
 *
 * The switch does not cut power itself. Sliding it off pulls GPIO6 low and
 * starts a timer in the m5ioe1 IO expander. The chip cuts power once the
 * time since the flip passes its OFF_TIME register: milliseconds, 16-bit
 * little-endian at 0xB2, back to 2000 at every power on. It compares with
 * the register as it is now, and a write does not restart the count.
 *
 * So at the flip this driver raises OFF_TIME to its maximum (65.5 s) and
 * asks userspace for an orderly poweroff. The screen stays lit: the shell
 * draws its shutdown screen as it stops. Once that
 * poweroff reaches the kernel (devices shut down, filesystems already
 * unmounted or read-only) it writes a small OFF_TIME and the chip cuts power
 * at once. If the shutdown is still running at fallback_ms, it syncs and
 * remounts read-only, so the timer's cut loses nothing.
 *
 * The node is M5's, from cardputerzero-v5-overlay, bound by its compatible.
 * Its disabled-gpios pin is on the m5ioe1, which is how this finds the chip.
 * Stock asserts that pin at the end, but it does not cut power: it is held
 * inactive, as stock holds it until then.
 */

#include <linux/atomic.h>
#include <linux/backlight.h>
#include <linux/delay.h>
#include <linux/gpio/consumer.h>
#include <linux/i2c.h>
#include <linux/interrupt.h>
#include <linux/module.h>
#include <linux/of.h>
#include <linux/platform_device.h>
#include <linux/reboot.h>
#include <linux/sysrq.h>
#include <linux/workqueue.h>

#define OFF_TIME_REG 0xb2
#define OFF_TIME_DEFAULT 2000
#define OFF_TIME_MAX 0xffff
/* Every path to the cut takes longer than this since the flip. */
#define OFF_TIME_CUT 1000

static unsigned int debounce_ms = 100;
module_param(debounce_ms, uint, 0644);
MODULE_PARM_DESC(debounce_ms, "How long GPIO6 must stay low to count as a flip");

static unsigned int fallback_ms = 55000;
module_param(fallback_ms, uint, 0644);
MODULE_PARM_DESC(fallback_ms, "When a shutdown still running gets sync and read-only");

static bool backlight_off;
module_param(backlight_off, bool, 0644);
MODULE_PARM_DESC(backlight_off, "Dark at the flip, as stock, instead of at the cut");

struct gh0st_power {
	struct device *dev;
	struct i2c_client *ioe;
	struct gpio_desc *sense;
	struct gpio_desc *disabled;
	struct backlight_device *backlight;
	atomic_t down;
	struct delayed_work fallback;
};

static int off_time_set(struct gh0st_power *p, u16 ms)
{
	int tries, ret = -EIO;

	/* One word write, as the timer was measured with, so it never sees half. */
	for (tries = 0; tries < 3; tries++) {
		ret = i2c_smbus_write_word_data(p->ioe, OFF_TIME_REG, ms);
		if (ret)
			continue;
		ret = i2c_smbus_read_word_data(p->ioe, OFF_TIME_REG);
		if (ret == ms)
			return 0;
		ret = ret < 0 ? ret : -EIO;
	}
	return ret;
}

static bool switch_off(struct gh0st_power *p)
{
	/* Raw level: low is off, whatever flags the node gives the pin. */
	return gpiod_get_raw_value_cansleep(p->sense) == 0;
}

static void read_only(void)
{
	handle_sysrq('s');
	handle_sysrq('u');
}

static irqreturn_t gh0st_power_irq(int irq, void *data)
{
	struct gh0st_power *p = data;
	int ret;

	if (atomic_read(&p->down))
		return IRQ_HANDLED;

	/* The chip counts from the edge: hold before the debounce. */
	ret = off_time_set(p, OFF_TIME_MAX);
	msleep(debounce_ms);
	if (!switch_off(p)) {
		if (!ret)
			off_time_set(p, OFF_TIME_DEFAULT);
		return IRQ_HANDLED;
	}
	atomic_set(&p->down, 1);

	if (p->backlight && backlight_off)
		backlight_device_set_brightness(p->backlight, 0);
	if (ret) {
		dev_emerg(p->dev, "switch off, power not held (%d): sync, read-only\n", ret);
		read_only();
		return IRQ_HANDLED;
	}
	dev_info(p->dev, "switch off: powering off\n");
	schedule_delayed_work(&p->fallback, msecs_to_jiffies(fallback_ms));
	orderly_poweroff(true);
	return IRQ_HANDLED;
}

static void gh0st_power_fallback(struct work_struct *work)
{
	struct gh0st_power *p = container_of(to_delayed_work(work), struct gh0st_power, fallback);

	dev_emerg(p->dev, "shutdown still running after %u ms: sync, read-only\n", fallback_ms);
	read_only();
}

/* Runs for poweroff and halt, after device_shutdown(), and may sleep. */
static int gh0st_power_off_prepare(struct sys_off_data *data)
{
	struct gh0st_power *p = data->cb_data;

	if (!atomic_read(&p->down))
		return NOTIFY_DONE;
	if (!switch_off(p)) {
		/* Flipped back on: the next flip gets the stock 2 s, not 65 s. */
		off_time_set(p, OFF_TIME_DEFAULT);
		return NOTIFY_DONE;
	}
	dev_emerg(p->dev, "cutting power\n");
	off_time_set(p, OFF_TIME_CUT);
	return NOTIFY_DONE;
}

static void put_ioe(void *data)
{
	put_device(data);
}

static void cancel_fallback(void *data)
{
	cancel_delayed_work_sync(data);
}

static int gh0st_power_probe(struct platform_device *pdev)
{
	struct device *dev = &pdev->dev;
	struct of_phandle_args args;
	struct gh0st_power *p;
	int irq, ret;

	p = devm_kzalloc(dev, sizeof(*p), GFP_KERNEL);
	if (!p)
		return -ENOMEM;
	p->dev = dev;
	atomic_set(&p->down, 0);
	INIT_DELAYED_WORK(&p->fallback, gh0st_power_fallback);
	ret = devm_add_action_or_reset(dev, cancel_fallback, &p->fallback);
	if (ret)
		return ret;

	p->sense = devm_gpiod_get(dev, "irq", GPIOD_IN);
	if (IS_ERR(p->sense))
		return dev_err_probe(dev, PTR_ERR(p->sense), "no switch gpio\n");
	p->disabled = devm_gpiod_get_optional(dev, "disabled", GPIOD_OUT_LOW);
	if (IS_ERR(p->disabled))
		return dev_err_probe(dev, PTR_ERR(p->disabled), "no disabled gpio\n");
	p->backlight = devm_of_find_backlight(dev);
	if (IS_ERR(p->backlight))
		return dev_err_probe(dev, PTR_ERR(p->backlight), "no backlight\n");

	ret = of_parse_phandle_with_args(dev->of_node, "disabled-gpios", "#gpio-cells", 0, &args);
	if (ret)
		return dev_err_probe(dev, ret, "no IO expander\n");
	p->ioe = of_find_i2c_device_by_node(args.np);
	of_node_put(args.np);
	if (!p->ioe)
		return -EPROBE_DEFER;
	ret = devm_add_action_or_reset(dev, put_ioe, &p->ioe->dev);
	if (ret)
		return ret;
	ret = i2c_smbus_read_word_data(p->ioe, OFF_TIME_REG);
	if (ret < 0)
		return dev_err_probe(dev, ret, "cannot read OFF_TIME\n");

	ret = devm_register_sys_off_handler(dev, SYS_OFF_MODE_POWER_OFF_PREPARE,
					    SYS_OFF_PRIO_DEFAULT, gh0st_power_off_prepare, p);
	if (ret)
		return ret;

	irq = platform_get_irq(pdev, 0);
	if (irq < 0)
		return irq;
	ret = devm_request_threaded_irq(dev, irq, NULL, gh0st_power_irq, IRQF_ONESHOT,
					"gh0st-power", p);
	if (ret)
		return dev_err_probe(dev, ret, "irq %d\n", irq);

	dev_info(dev, "power switch ready, OFF_TIME %d ms\n",
		 i2c_smbus_read_word_data(p->ioe, OFF_TIME_REG));
	return 0;
}

static const struct of_device_id gh0st_power_of_match[] = {
	{ .compatible = "cardputerzero,powerfail-suo" },
	{ }
};
MODULE_DEVICE_TABLE(of, gh0st_power_of_match);

static struct platform_driver gh0st_power_driver = {
	.probe = gh0st_power_probe,
	.driver = {
		.name = "gh0st-power",
		.of_match_table = gh0st_power_of_match,
	},
};
module_platform_driver(gh0st_power_driver);

MODULE_DESCRIPTION("CardputerZero power switch: an orderly poweroff, then the cut");
MODULE_LICENSE("GPL");
