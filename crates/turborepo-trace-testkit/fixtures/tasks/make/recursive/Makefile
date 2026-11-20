SUBDIRS := lib app

.PHONY: all clean $(SUBDIRS)

all: $(SUBDIRS)

app: lib

$(SUBDIRS):
	$(MAKE) -C $@

test: all
	$(MAKE) -C app test

clean:
	for d in $(SUBDIRS); do $(MAKE) -C $$d clean; done
