include ../mk/common.mk

.PHONY: all test clean
all: app

app: main.o ../lib/liblib.a
	$(CC) -o $@ main.o -L../lib -llib

test: app
	./app --self-test

clean:
	$(RM) app main.o
