KERNELDIR ?= /lib/modules/$(shell uname -r)/build
obj-m := gh0st-power.o

all:
	$(MAKE) -C $(KERNELDIR) M=$(CURDIR) modules

clean:
	$(MAKE) -C $(KERNELDIR) M=$(CURDIR) clean
