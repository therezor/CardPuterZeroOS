# Out-of-tree build against the running kernel's headers, on the device.
KERNELDIR ?= /lib/modules/$(shell uname -r)/build
obj-m := panel-mipi-dbi-m.o

all:
	$(MAKE) -C $(KERNELDIR) M=$(CURDIR) modules

clean:
	$(MAKE) -C $(KERNELDIR) M=$(CURDIR) clean
