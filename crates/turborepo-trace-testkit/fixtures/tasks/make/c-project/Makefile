CC      = gcc
CFLAGS  = -Wall -O2 -Iinclude
LDLIBS  = -lm
SRCS   := $(wildcard src/*.c)
OBJS   := $(patsubst src/%.c,build/%.o,$(SRCS))
DEPS   := $(OBJS:.o=.d)

.PHONY: all clean check

all: app

app: $(OBJS)
	$(CC) -o $@ $^ $(LDLIBS)

build/%.o: src/%.c | build
	$(CC) $(CFLAGS) -MMD -MF build/$*.d -c $< -o $@

build:
	mkdir -p $@

libutil.a: build/util.o
	$(AR) rcs $@ $^

# Old-style suffix rule and the built-in %.o: %.c rule.
.SUFFIXES: .txt .html
.txt.html:
	cp $< $@

docs: notes.html

check: app
	./app

clean:
	$(RM) -r build app libutil.a

-include $(DEPS)
