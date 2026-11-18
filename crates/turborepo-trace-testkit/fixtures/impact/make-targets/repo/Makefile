# Generated sources consumed by the tests (src/generated is gitignored).
GEN  := src/generated
NODE ?= node

.PHONY: all gen docs clean

all: gen

gen: $(GEN)/index.js

$(GEN)/schema.js: schema/schema.json scripts/gen-schema.js | $(GEN)
	$(NODE) scripts/gen-schema.js

$(GEN)/version.js: version.txt scripts/gen-version.js | $(GEN)
	$(NODE) scripts/gen-version.js

# Depends on the two targets above.
$(GEN)/index.js: $(GEN)/schema.js $(GEN)/version.js scripts/gen-index.js
	$(NODE) scripts/gen-index.js

$(GEN):
	mkdir -p $@

docs: docs/index.html

docs/index.html: docs/src/index.md
	cp $< $@

clean:
	rm -rf $(GEN) docs/index.html
