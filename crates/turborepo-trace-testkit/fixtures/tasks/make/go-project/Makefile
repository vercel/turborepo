# A typical Go service Makefile.
BINARY  := bin/server
VERSION ?= $(shell git describe --tags --always --dirty)
GO      ?= go
GOFILES := $(wildcard cmd/server/*.go) $(wildcard internal/*/*.go)
LDFLAGS := -X main.version=$(VERSION)

.PHONY: all build test generate lint assets image clean
.DEFAULT_GOAL := build

all: generate build test

build: $(BINARY)

$(BINARY): $(GOFILES) go.mod go.sum
	@mkdir -p $(@D)
	$(GO) build -ldflags "$(LDFLAGS)" -o $@ ./cmd/server

test: generate
	$(GO) test -race ./...

generate: internal/api/gen.go

internal/api/gen.go: api/openapi.yaml scripts/gen.mjs
	node scripts/gen.mjs $< > $@

ifeq ($(CI),true)
LINTFLAGS := --out-format github-actions
endif

lint:
	golangci-lint run $(LINTFLAGS) ./...

assets:
	npm run build

image: build
	docker build -t server:$(VERSION) .

clean:
	rm -rf bin
