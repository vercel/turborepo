include ../mk/common.mk
-include local.mk

.PHONY: all clean
all: liblib.a

liblib.a: lib.o
	$(AR) rcs $@ $^

clean:
	$(RM) liblib.a lib.o
