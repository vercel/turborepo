
TURBO_VERSION = $(shell head -n 1 version.txt)
TURBO_TAG = $(shell cat version.txt | sed -n '2 p')

# Strip debug info
GO_FLAGS += "-ldflags=-s -w"

# Avoid embedding the build path in the executable for more reproducible builds
GO_FLAGS += -trimpath

GO_FILES = $(shell find . -name "*.go")
SRC_FILES = $(shell find . -name "*.go" | grep -v "_test.go")
GENERATED_FILES = internal/turbodprotocol/turbod.pb.go internal/turbodprotocol/turbod_grpc.pb.go

turbo: $(GENERATED_FILES) $(SRC_FILES) go.mod
	CGO_ENABLED=0 go build $(GO_FLAGS) ./cmd/turbo

protoc: internal/turbodprotocol/turbod.proto
	protoc --go_out=. --go_opt=paths=source_relative \
    --go-grpc_out=. --go-grpc_opt=paths=source_relative \
    internal/turbodprotocol/turbod.proto

$(GENERATED_FILES): internal/turbodprotocol/turbod.proto
	make protoc

compile-protos: $(GENERATED_FILES)

ewatch: scripts/...
	nodemon --exec "make e2e" -e .ts,.go

check-go-version:
	@go version | grep ' go1\.18\.0 ' || (echo 'Please install Go version 1.18.0' && false)

# This "TURBO_RACE" variable exists at the request of a user on GitHub who
# wants to run "make test-go" on an unsupported version of macOS (version 10.9).
# Go's race detector does not run correctly on that version. With this flag
# you can run "TURBO_RACE= make test-go" to disable the race detector.
TURBO_RACE ?= -race

clean-go:
	go clean -testcache ./...

test-go:$(GENERATED_FILES) $(GO_FILES) go.mod go.sum
	go test $(TURBO_RACE) ./...

# protos need to be compiled before linting, since linting needs to pick up
# some types from the generated code
lint-go: $(GENERATED_FILES) $(GO_FILES) go.mod go.sum
	golangci-lint run --new-from-rev=main

fmt-go: $(GO_FILES) go.mod go.sum
	go fmt ./...

install: | ./package.json
	pnpm install --filter cli

e2e: install turbo
	node -r esbuild-register scripts/e2e/e2e.ts

cmd/turbo/version.go: version.txt
	# Update this atomically to avoid issues with this being overwritten during use
	node -e 'console.log(`package main\n\nconst turboVersion = "$(TURBO_VERSION)"`)' > cmd/turbo/version.go.txt
	mv cmd/turbo/version.go.txt cmd/turbo/version.go

platform-all: cmd/turbo/version.go $(GENERATED_FILES)
	make -j4 \
	  platform-windows-64 \
		platform-windows-32 \
		platform-windows-arm64 \
		platform-android-arm64 \
		platform-darwin-64 \
		platform-darwin-arm64 \
		platform-freebsd-64 \
		platform-freebsd-arm64 \
		platform-netbsd \
		platform-openbsd \
		platform-linux-64 \
		platform-linux-32 \
		platform-linux-arm \
		platform-linux-arm64 \
		platform-linux-mips64le \
		platform-linux-ppc64le \
		platform-linux-s390x \
		platform-sunos \
		platform-neutral \
		platform-create-turbo


platform-windows-32:
	cd npm/turbo-windows-32 && npm version "$(TURBO_VERSION)" --allow-same-version
	CGO_ENABLED=0 GOOS=windows GOARCH=386 go build $(GO_FLAGS) -o npm/turbo-windows-32/turbo.exe ./cmd/turbo

platform-windows-64:
	cd npm/turbo-windows-64 && npm version "$(TURBO_VERSION)" --allow-same-version
	CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build $(GO_FLAGS) -o npm/turbo-windows-64/turbo.exe ./cmd/turbo

platform-windows-arm64:
	cd npm/turbo-windows-arm64 && npm version "$(TURBO_VERSION)" --allow-same-version
	CGO_ENABLED=0 GOOS=windows GOARCH=arm64 go build $(GO_FLAGS) -o npm/turbo-windows-arm64/turbo.exe ./cmd/turbo

platform-unixlike:
	test -n "$(GOOS)" && test -n "$(GOARCH)" && test -n "$(NPMDIR)"
	mkdir -p "$(NPMDIR)/bin"
	cd "$(NPMDIR)" && npm version "$(TURBO_VERSION)" --allow-same-version
	CGO_ENABLED=0 GOOS="$(GOOS)" GOARCH="$(GOARCH)" go build $(GO_FLAGS)  -o "$(NPMDIR)/bin/turbo" ./cmd/turbo

# To fix https://github.com/vercel/turborepo/issues/941 (DNS issue),
# we compile the darwin-arm64 binary with CGO_ENABLED=1 on an macOS M1 (darwin-arm64)
platform-unixlike-cgo:
	test -n "$(GOOS)" && test -n "$(GOARCH)" && test -n "$(NPMDIR)"
	mkdir -p "$(NPMDIR)/bin"
	cd "$(NPMDIR)" && npm version "$(TURBO_VERSION)" --allow-same-version
	CGO_ENABLED=1 GOOS="$(GOOS)" GOARCH="$(GOARCH)" go build $(GO_FLAGS)  -o "$(NPMDIR)/bin/turbo" ./cmd/turbo

platform-android-arm64:
	make GOOS=android GOARCH=arm64 NPMDIR=npm/turbo-android-arm64 platform-unixlike

platform-darwin-64:
	make GOOS=darwin GOARCH=amd64 NPMDIR=npm/turbo-darwin-64 platform-unixlike

platform-darwin-arm64:
	make GOOS=darwin GOARCH=arm64 NPMDIR=npm/turbo-darwin-arm64 platform-unixlike-cgo

platform-freebsd-64:
	make GOOS=freebsd GOARCH=amd64 NPMDIR=npm/turbo-freebsd-64 platform-unixlike

platform-freebsd-arm64:
	make GOOS=freebsd GOARCH=arm64 NPMDIR=npm/turbo-freebsd-arm64 platform-unixlike

platform-netbsd:
	make GOOS=netbsd GOARCH=amd64 NPMDIR=npm/turbo-netbsd-64 platform-unixlike

platform-openbsd:
	make GOOS=openbsd GOARCH=amd64 NPMDIR=npm/turbo-openbsd-64 platform-unixlike

platform-linux-64:
	make GOOS=linux GOARCH=amd64 NPMDIR=npm/turbo-linux-64 platform-unixlike

platform-linux-32:
	make GOOS=linux GOARCH=386 NPMDIR=npm/turbo-linux-32 platform-unixlike

platform-linux-arm:
	make GOOS=linux GOARCH=arm NPMDIR=npm/turbo-linux-arm platform-unixlike

platform-linux-arm64:
	make GOOS=linux GOARCH=arm64 NPMDIR=npm/turbo-linux-arm64 platform-unixlike

platform-linux-mips64le:
	make GOOS=linux GOARCH=mips64le NPMDIR=npm/turbo-linux-mips64le platform-unixlike

platform-linux-ppc64le:
	make GOOS=linux GOARCH=ppc64le NPMDIR=npm/turbo-linux-ppc64le platform-unixlike

platform-linux-s390x:
	make GOOS=linux GOARCH=s390x NPMDIR=npm/turbo-linux-s390x platform-unixlike

platform-sunos:
	# make GOOS=illumos GOARCH=amd64 NPMDIR=npm/turbo-sunos-64 platform-unixlike
	@echo skipping illumos, not supported for now

platform-neutral: install
	cd npm/turbo-install && npm version "$(TURBO_VERSION)" --allow-same-version
	node ./scripts/bump-version.js

platform-create-turbo:
	cd ../packages/create-turbo && npm version "$(TURBO_VERSION)" --allow-same-version && npm run build

platform-turbo-codemod:
	cd ../packages/turbo-codemod && npm version "$(TURBO_VERSION)" --allow-same-version && npm run build

test-prepublish: clean-demo
	make -j3 bench/turbo test-go lint-go e2e


publish-all: cmd/turbo/version.go
	# @test main = "`git rev-parse --abbrev-ref HEAD`" || (echo "Refusing to publish from non-master branch `git rev-parse --abbrev-ref HEAD`" && false)
	@echo "Checking for unpushed commits..." && git fetch
	@test "" = "`git cherry`" || (echo "Refusing to publish with unpushed commits" && false)
	rm -fr npm && git checkout npm
	@echo Enter one-time password:
	make publish-windows-32
	make publish-windows-64
	make publish-windows-arm64
	make publish-freebsd-64
	make publish-freebsd-arm64
	@echo Enter one-time password:
	make -j4 \
		publish-darwin-64 \
		publish-darwin-arm64 \
		publish-linux-64 \
		publish-linux-32
	@echo Enter one-time password:
	make -j4 \
		publish-linux-arm \
		publish-linux-arm64 \
		publish-android-arm64 \
		publish-linux-mips64le \
		publish-linux-ppc64le
	# Do these last to avoid race conditions
	@echo Confirm release:
	make publish-neutral
	make publish-create-turbo
	make publish-turbo-codemod
	git commit -am "publish $(TURBO_VERSION) to registry"
	git tag "v$(TURBO_VERSION)"
	git push origin main "v$(TURBO_VERSION)"

publish-windows-32: test-go lint-go platform-windows-32
	cd npm/turbo-windows-32 && npm publish --tag $(TURBO_TAG)

publish-windows-64: test-go lint-go platform-windows-64
	cd npm/turbo-windows-64 && npm publish --tag $(TURBO_TAG)

publish-windows-arm64: test-go lint-go platform-windows-arm64
	cd npm/turbo-windows-arm64 && npm publish --tag $(TURBO_TAG)

publish-darwin-64: test-go lint-go platform-darwin-64
	cd npm/turbo-darwin-64 && npm publish --tag $(TURBO_TAG)

publish-darwin-arm64: test-go lint-go platform-darwin-arm64
	cd npm/turbo-darwin-arm64 && npm publish --tag $(TURBO_TAG)

publish-freebsd-64: test-go lint-go platform-freebsd-64
	cd npm/turbo-freebsd-64 && npm publish --tag $(TURBO_TAG)

publish-freebsd-arm64: test-go lint-go platform-freebsd-arm64
	cd npm/turbo-freebsd-arm64 && npm publish --tag $(TURBO_TAG)

publish-linux-64: test-go lint-go platform-linux-64
	cd npm/turbo-linux-64 && npm publish --tag $(TURBO_TAG)

publish-linux-32: test-go lint-go platform-linux-32
	cd npm/turbo-linux-32 && npm publish --tag $(TURBO_TAG)

publish-linux-arm: test-go lint-go platform-linux-arm
	cd npm/turbo-linux-arm && npm publish --tag $(TURBO_TAG)

publish-linux-arm64: test-go lint-go platform-linux-arm64
	cd npm/turbo-linux-arm64 && npm publish --tag $(TURBO_TAG)

publish-android-arm64: test-go lint-go platform-android-arm64
	cd npm/turbo-android-arm64 && npm publish --tag $(TURBO_TAG)

publish-linux-mips64le: test-go lint-go platform-linux-mips64le
	cd npm/turbo-linux-mips64le && npm publish --tag $(TURBO_TAG)

publish-linux-ppc64le: test-go lint-go platform-linux-ppc64le
	cd npm/turbo-linux-ppc64le && npm publish --tag $(TURBO_TAG)

publish-neutral: test-go lint-go platform-neutral
	cd npm/turbo-install && npm publish --tag $(TURBO_TAG)

publish-create-turbo: platform-create-turbo
	cd ../packages/create-turbo && npm publish --tag $(TURBO_TAG)

publish-turbo-codemod: platform-turbo-codemod
	cd ../packages/turbo-codemod && npm publish --tag $(TURBO_TAG)

demo/lage: install
	node scripts/generate.mjs lage

demo/lerna: install
	node scripts/generate.mjs lerna

demo/nx: install
	node scripts/generate.mjs nx

demo/turbo: install
	node scripts/generate.mjs turbo

demo: demo/lage demo/lerna demo/nx demo/turbo

bench/lerna: demo/lerna
	cd demo/lerna && node_modules/.bin/lerna run build

bench/lage: demo/lage
	cd demo/lage && node_modules/.bin/lage build

bench/nx: demo/nx
	cd demo/nx && node_modules/.bin/nx run-many --target=build --all

bench/turbo: demo/turbo turbo
	cd demo/turbo && ../../turbo run test

bench: bench/lerna bench/lage bench/nx bench/turbo

clean: clean-go clean-build clean-demo

clean-build:
	rm -f turbo
	rm -f npm/turbo-android-arm64/bin/turbo
	rm -f npm/turbo-darwin-64/bin/turbo
	rm -f npm/turbo-darwin-arm64/bin/turbo
	rm -f npm/turbo-freebsd-64/bin/turbo
	rm -f npm/turbo-freebsd-arm64/bin/turbo
	rm -f npm/turbo-linux-32/bin/turbo
	rm -f npm/turbo-linux-64/bin/turbo
	rm -f npm/turbo-linux-arm/bin/turbo
	rm -f npm/turbo-linux-arm64/bin/turbo
	rm -f npm/turbo-linux-mips64le/bin/turbo
	rm -f npm/turbo-linux-ppc64le/bin/turbo
	rm -f npm/turbo-linux-s390x/bin/turbo
	rm -f npm/turbo-netbsd-64/bin/turbo
	rm -f npm/turbo-openbsd-64/bin/turbo
	rm -f npm/turbo-sunos-64/bin/turbo
	rm -f npm/turbo-windows-32/turbo.exe
	rm -f npm/turbo-windows-64/turbo.exe
	rm -f npm/turbo-windows-arm64/turbo.exe

clean-demo:
	rm -rf node_modules
	rm -rf demo
