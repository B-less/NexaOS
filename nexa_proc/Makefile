obj-m += nexa_proc.o
KDIR := /home/nexa/NexaOS/kernel/linux-6.18.33
PWD  := $(shell pwd)

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean
